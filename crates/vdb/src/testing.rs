//! Honest, configurable virtual database fixtures for integration tests.

use std::{
    collections::BTreeMap,
    future::{Ready, ready},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use semantic_data::{
    Object,
    query::{Expr, Query, SelectQuery, SortDirection},
    schema::{
        AttributeRef, AttributeType, ClassAttribute, ClassType, Meta, NumberType, StringType, Type,
        TypeKind,
    },
};
use semantic_db_core::{catalog::Catalog, evaluate_expr, evaluate_filter_expr};
use semantic_plugin::{PluginInstanceContext, PluginManifest};
use tokio::sync::Notify;

use crate::{
    AcceptedScan, CancellationToken, DatabaseDescriptor, DatabaseSchema, EntityStream,
    FilterSupport, ScanPlan, ScanRequest, VdbError, VirtualDatabase, VirtualDatabasePlugin,
    implementation_descriptor,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NegotiationMode {
    AllExact,
    AllInexact,
    AllUnsupported,
    Alternating,
}

pub type SchemaHandle = Arc<Mutex<(DatabaseSchema, String)>>;

#[derive(Clone)]
pub struct FixtureVdb {
    pub entities: Vec<Object>,
    pub mode: NegotiationMode,
    pub reject_without: Option<String>,
    pub scans: Arc<AtomicUsize>,
    pub describes: Arc<AtomicUsize>,
    pub activations: Arc<AtomicUsize>,
    pub cancelled: Arc<Notify>,
    pub schema: SchemaHandle,
    pub scan_delay: Duration,
}

impl FixtureVdb {
    pub fn new(entities: Vec<Object>, mode: NegotiationMode) -> Self {
        Self {
            entities,
            mode,
            reject_without: None,
            scans: Arc::new(AtomicUsize::new(0)),
            describes: Arc::new(AtomicUsize::new(0)),
            activations: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(Notify::new()),
            schema: Arc::new(Mutex::new((fixture_schema(), "1".into()))),
            scan_delay: Duration::ZERO,
        }
    }

    pub fn with_schema(mut self, schema: DatabaseSchema, revision: impl Into<String>) -> Self {
        self.schema = Arc::new(Mutex::new((schema, revision.into())));
        self
    }

    pub fn schema_handle(&self) -> SchemaHandle {
        self.schema.clone()
    }

    pub fn with_reject_without(mut self, field: impl Into<String>) -> Self {
        self.reject_without = Some(field.into());
        self
    }

    pub fn with_scan_delay(mut self, delay: Duration) -> Self {
        self.scan_delay = delay;
        self
    }
}

#[async_trait]
impl VirtualDatabase for FixtureVdb {
    async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
        self.describes.fetch_add(1, Ordering::SeqCst);
        let (schema, schema_revision) = self.schema.lock().unwrap().clone();
        Ok(DatabaseDescriptor {
            title: "Fixture database".into(),
            description: Some("In-memory entities for federation tests".into()),
            schema,
            schema_revision,
            allow_untyped: true,
        })
    }

    fn schema_revision(&self) -> String {
        self.schema.lock().unwrap().1.clone()
    }

    async fn negotiate(&self, request: &ScanRequest) -> Result<ScanPlan, VdbError> {
        if let Some(field) = &self.reject_without {
            // Inspect the AST, rather than its debug rendering, including IN
            // parameters used by bind joins.
            let references = request.filters.iter().any(|expr| {
                let mut query = Query::Select(SelectQuery {
                    predicate: Some(expr.clone()),
                    ..Default::default()
                });
                let mut found = false;
                query
                    .visit_expressions_mut(&mut |expr| {
                        if let Expr::Operand(semantic_data::query::Operand::Field(path)) = expr {
                            found |= *path
                                == semantic_data::value::FieldPath::from_fields([field.as_str()]);
                        }
                        Ok::<(), std::convert::Infallible>(())
                    })
                    .unwrap();
                found
            });
            if !references {
                return Ok(ScanPlan::Rejected {
                    reason: format!("scan requires a filter on '{field}'"),
                });
            }
        }
        let filters: Vec<_> = (0..request.filters.len())
            .map(|i| match self.mode {
                NegotiationMode::AllExact => FilterSupport::Exact,
                NegotiationMode::AllInexact => FilterSupport::Inexact,
                NegotiationMode::AllUnsupported => FilterSupport::Unsupported,
                NegotiationMode::Alternating => [
                    FilterSupport::Exact,
                    FilterSupport::Inexact,
                    FilterSupport::Unsupported,
                ][i % 3],
            })
            .collect();
        let ordered_prefix = if self.mode == NegotiationMode::AllExact {
            request.order_by.len() as u64
        } else {
            0
        };
        let can_slice = filters
            .iter()
            .all(|support| *support == FilterSupport::Exact)
            && ordered_prefix == request.order_by.len() as u64
            && matches!(
                self.mode,
                NegotiationMode::AllExact | NegotiationMode::Alternating
            );
        Ok(ScanPlan::Accepted {
            plan: AcceptedScan {
                filters,
                ordered_prefix,
                limit_applied: can_slice && request.limit.is_some(),
                offset_applied: can_slice && request.offset > 0,
                estimated_rows: Some(self.entities.len() as u64),
                token: None,
                schema_revision: self.schema_revision(),
            },
        })
    }

    fn scan(
        &self,
        request: ScanRequest,
        plan: AcceptedScan,
        bindings: Object,
        cancellation: CancellationToken,
    ) -> EntityStream {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let mut rows = self.entities.clone();
        let delay = self.scan_delay;
        let guard = CancelledScan {
            notification: self.cancelled.clone(),
            completed: false,
        };
        Box::pin(async_stream::try_stream! {
            let mut guard = guard;
            plan.validate(&request)?;
            let predicate = request.filters.iter().zip(&plan.filters)
                .filter(|(_, support)| **support == FilterSupport::Exact)
                .map(|(filter, _)| filter.clone())
                .reduce(|left, right| Expr::Binary { op: semantic_data::query::BinaryOp::And, left: Box::new(left), right: Box::new(right) });
            let query = Query::Select(SelectQuery { predicate, order_by: request.order_by[..plan.ordered_prefix as usize].to_vec(), ..Default::default() });
            let Query::Select(query) = query.into_bound(&bindings.into_btree()).map_err(|error| VdbError { code: "invalid_binding".into(), message: error.to_string() })? else { unreachable!() };
            let query: semantic_db_core::SelectQuery = query.into();
            if let Some(predicate) = query.predicate {
                rows.retain(|row| evaluate_filter_expr(row, &predicate));
            }
            rows.sort_by(|a, b| {
                for order in &query.order_by {
                    let ordering = evaluate_expr(a, &order.expr).cmp(&evaluate_expr(b, &order.expr));
                    if ordering != std::cmp::Ordering::Equal {
                        return if order.direction == SortDirection::Asc { ordering } else { ordering.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
            let offset = if plan.offset_applied { usize::try_from(request.offset).unwrap_or(usize::MAX) } else { 0 };
            let limit = if plan.limit_applied { usize::try_from(request.limit.unwrap()).unwrap_or(usize::MAX) } else { usize::MAX };
            for row in rows.into_iter().skip(offset).take(limit) {
                let cancelled = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => true,
                    _ = tokio::time::sleep(delay) => false,
                };
                if cancelled {
                    Err(VdbError { code: "cancelled".into(), message: "fixture scan cancelled".into() })?;
                }
                yield row;
            }
            guard.completed = true;
        })
    }
}

struct CancelledScan {
    notification: Arc<Notify>,
    completed: bool,
}

impl Drop for CancelledScan {
    fn drop(&mut self) {
        if !self.completed {
            self.notification.notify_one();
        }
    }
}

pub fn fixture_plugin(
    id: &str,
    entities: Vec<Object>,
    mode: NegotiationMode,
) -> VirtualDatabasePlugin<
    impl Fn(PluginInstanceContext) -> Ready<Result<FixtureVdb, VdbError>> + Clone,
> {
    fixture_plugin_with_vdb(id, FixtureVdb::new(entities, mode))
}

pub fn fixture_plugin_with_vdb(
    id: &str,
    database: FixtureVdb,
) -> VirtualDatabasePlugin<
    impl Fn(PluginInstanceContext) -> Ready<Result<FixtureVdb, VdbError>> + Clone,
> {
    let mut catalog = Catalog::new();
    catalog.upsert_package(semantic_data::bundles::query::package());
    catalog.upsert_package(crate::package());
    VirtualDatabasePlugin::new(
        PluginManifest {
            id: id.into(),
            revision: "1".into(),
            title: "Fixture database".into(),
            exports: vec![
                implementation_descriptor(&catalog, "database")
                    .expect("fixture interface is registered"),
            ],
            configuration_schema: None,
            source_bindings: BTreeMap::new(),
        },
        move |_context| {
            database.activations.fetch_add(1, Ordering::SeqCst);
            ready(Ok(database.clone()))
        },
    )
}

pub fn fixture_schema() -> DatabaseSchema {
    let string = Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }));
    let attributes = [
        ("fixture:name", "name", string.clone()),
        ("fixture:category", "category", string),
        (
            "fixture:score",
            "score",
            Type::new(TypeKind::Number(NumberType::Unspecified)),
        ),
        ("fixture:active", "active", Type::new_bool()),
    ]
    .into_iter()
    .map(|(id, name, ty)| AttributeType {
        id: id.into(),
        name: name.into(),
        ty,
        constraints: vec![],
        meta: Meta::default(),
    })
    .collect();
    let classes = [("fixture:Item", "Item"), ("fixture:Tag", "Tag")]
        .into_iter()
        .map(|(id, name)| ClassType {
            id: id.into(),
            name: name.into(),
            inherits: None,
            extends: vec![],
            strict_schema: true,
            creatable_in_ui: None,
            include_in_ui_listings: None,
            attributes: [
                ("name", "fixture:name"),
                ("category", "fixture:category"),
                ("score", "fixture:score"),
                ("active", "fixture:active"),
                ("title", semantic_data::attr::ATTR_TITLE),
            ]
            .into_iter()
            .map(|(name, id)| {
                (
                    name.into(),
                    ClassAttribute {
                        attribute: AttributeRef { id: id.into() },
                        required: false,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
            constraints: vec![],
            meta: Meta::default(),
        })
        .collect();
    DatabaseSchema {
        attributes,
        classes,
        ..Default::default()
    }
}

pub fn broken_fixture_schema() -> DatabaseSchema {
    let mut schema = fixture_schema();
    schema.classes[0]
        .attributes
        .get_mut("name")
        .unwrap()
        .attribute
        .id = "fixture:missing".into();
    schema
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use semantic_data::{
        Value,
        query::{BinaryOp, Operand, OrderBy},
        value::FieldPath,
    };

    use super::*;

    fn field(name: &str) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
    }

    fn equals(name: &str, value: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(name)),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(value.into())))),
        }
    }

    fn rows() -> Vec<Object> {
        ["three", "two", "one"]
            .into_iter()
            .map(|id| {
                let mut row = Object::new();
                row.insert("id", Value::String(id.into()));
                row.insert("type", Value::String("fixture:Item".into()));
                row
            })
            .collect()
    }

    fn request() -> ScanRequest {
        ScanRequest {
            filters: vec![equals("type", "fixture:Item"), equals("id", "one")],
            order_by: vec![OrderBy {
                expr: field("id"),
                direction: SortDirection::Asc,
            }],
            limit: Some(1),
            offset: 0,
            projection: None,
            parameters: vec![],
            fetch_hint: None,
        }
    }

    async fn smoke(mode: NegotiationMode) {
        let fixture = FixtureVdb::new(rows(), mode);
        let request = request();
        let ScanPlan::Accepted { plan } = fixture.negotiate(&request).await.unwrap() else {
            panic!("accepted")
        };
        plan.validate(&request).unwrap();
        let supports = plan.filters.clone();
        let stream = fixture.scan(
            request.clone(),
            plan,
            Object::new(),
            CancellationToken::new(),
        );
        let scanned = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (filter, support) in request.filters.iter().zip(supports) {
            if support == FilterSupport::Exact {
                assert!(
                    scanned
                        .iter()
                        .all(|row| evaluate_filter_expr(row, &filter.clone().into()))
                );
            }
        }
        assert_eq!(
            scanned.len(),
            if mode == NegotiationMode::AllExact {
                1
            } else {
                3
            }
        );
        assert_eq!(fixture.scans.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.describe().await.unwrap().schema_revision, "1");
    }

    #[tokio::test]
    async fn all_exact() {
        smoke(NegotiationMode::AllExact).await;
    }
    #[tokio::test]
    async fn all_inexact() {
        smoke(NegotiationMode::AllInexact).await;
    }
    #[tokio::test]
    async fn all_unsupported() {
        smoke(NegotiationMode::AllUnsupported).await;
    }
    #[tokio::test]
    async fn alternating() {
        smoke(NegotiationMode::Alternating).await;
    }

    #[tokio::test]
    async fn exact_sort_offset_and_bound_filter() {
        let fixture = FixtureVdb::new(rows(), NegotiationMode::AllExact);
        let mut request = request();
        request.filters = vec![Expr::Binary {
            op: BinaryOp::In,
            left: Box::new(field("id")),
            right: Box::new(Expr::parameter("keys")),
        }];
        request.parameters = vec!["keys".into()];
        request.offset = 1;
        request.order_by[0].direction = SortDirection::Desc;
        let ScanPlan::Accepted { plan } = fixture.negotiate(&request).await.unwrap() else {
            panic!("accepted")
        };
        let mut bindings = Object::new();
        bindings.insert(
            "keys",
            Value::List(vec![
                Value::String("one".into()),
                Value::String("two".into()),
                Value::String("three".into()),
            ]),
        );
        let scanned = fixture
            .scan(request, plan, bindings, CancellationToken::new())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(scanned.len(), 1);
        assert_eq!(
            scanned[0].as_ref().unwrap().get("id"),
            Some(&Value::String("three".into()))
        );
    }

    #[tokio::test]
    async fn schema_revision_and_rejection() {
        let fixture =
            FixtureVdb::new(rows(), NegotiationMode::AllExact).with_reject_without("type");
        let mut request = request();
        request.filters.clear();
        assert!(
            matches!(fixture.negotiate(&request).await.unwrap(), ScanPlan::Rejected { reason } if reason.contains("type"))
        );
        let handle = fixture.schema_handle();
        *handle.lock().unwrap() = (broken_fixture_schema(), "2".into());
        assert_eq!(fixture.schema_revision(), "2");
        assert_eq!(
            fixture.describe().await.unwrap().schema,
            broken_fixture_schema()
        );
        assert_eq!(fixture.describes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_and_dropping_unpolled_scans() {
        let fixture = FixtureVdb::new(rows(), NegotiationMode::AllExact)
            .with_scan_delay(Duration::from_secs(60));
        let request = request();
        let ScanPlan::Accepted { plan } = fixture.negotiate(&request).await.unwrap() else {
            panic!("accepted")
        };
        let cancelled = fixture.cancelled.notified();
        drop(fixture.scan(
            request.clone(),
            plan.clone(),
            Object::new(),
            CancellationToken::new(),
        ));
        tokio::time::timeout(Duration::from_secs(1), cancelled)
            .await
            .unwrap();
        let token = CancellationToken::new();
        let mut stream = fixture.scan(request, plan, Object::new(), token.clone());
        token.cancel();
        assert_eq!(stream.next().await.unwrap().unwrap_err().code, "cancelled");
    }
}
