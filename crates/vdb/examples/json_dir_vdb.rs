//! One virtual entity per JSON file in a directory (without recursion).
//!
//! Build: `cargo build -p semantic_vdb --example json_dir_vdb`.
//! Run in process: `target/debug/examples/json_dir_vdb ./documents`.
//! Run as a stdio plugin: `target/debug/examples/json_dir_vdb ./documents --stdio`.
//! Generate a portable activation: `json_dir_vdb ./documents --activation fx`.
//! Pass that JSON to `semantic api plugin configure`; query `SELECT * FROM fx`.
//!
//! File names (including `.json`) are stable ids. The complete JSON value is
//! carried by `json_dir:content`; the class is `json_dir:Document`. The fixed
//! schema has revision "1"; changing file contents does not change the schema.

use std::{collections::BTreeMap, path::PathBuf};

use async_trait::async_trait;
use futures_util::StreamExt;
use semantic_data::query::{BinaryOp, Expr, Operand, SortDirection};
use semantic_data::schema::{
    AnyType, AttributeRef, AttributeType, ClassAttribute, ClassType, Meta, Type, TypeKind,
};
use semantic_data::value::{FieldPath, PathSegment};
use semantic_data::{Object, Value};
use semantic_db_core::catalog::Catalog;
use semantic_plugin::{
    Plugin, PluginActivation, PluginInstanceContext, PluginManifest, PluginProvider,
    portable_export,
};
use semantic_vdb::{
    AcceptedScan, CancellationToken, DatabaseDescriptor, DatabaseSchema, EntityStream,
    FilterSupport, ScanPlan, ScanRequest, VdbError, VirtualDatabase, VirtualDatabasePlugin,
    classify_simple, implementation_descriptor,
};

const CLASS: &str = "json_dir:Document";
const CONTENT: &str = "json_dir:content";
const REVISION: &str = "1";

#[derive(Clone)]
struct JsonDirVdb {
    directory: PathBuf,
}

fn exposed_schema() -> DatabaseSchema {
    DatabaseSchema {
        attributes: vec![AttributeType {
            id: CONTENT.into(),
            name: "content".into(),
            ty: Type::new(TypeKind::Any(AnyType)),
            constraints: vec![],
            meta: Meta::default(),
        }],
        classes: vec![ClassType {
            id: CLASS.into(),
            name: "Document".into(),
            inherits: None,
            extends: vec![],
            strict_schema: true,
            creatable_in_ui: None,
            include_in_ui_listings: None,
            attributes: BTreeMap::from([(
                "content".into(),
                ClassAttribute {
                    attribute: AttributeRef { id: CONTENT.into() },
                    required: true,
                    ui_order: None,
                    computed: None,
                    default: None,
                    constraints: vec![],
                    meta: Meta::default(),
                },
            )]),
            constraints: vec![],
            meta: Meta::default(),
        }],
        ..DatabaseSchema::default()
    }
}

fn field_is(path: &FieldPath, field: &str) -> bool {
    matches!(path.segments(), [PathSegment::Field(name)] if name == field)
}

fn failure(error: impl std::fmt::Display) -> VdbError {
    VdbError {
        code: "json_directory_error".into(),
        message: error.to_string(),
    }
}

#[async_trait]
impl VirtualDatabase for JsonDirVdb {
    async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
        Ok(DatabaseDescriptor {
            title: "JSON directory".into(),
            description: Some(self.directory.display().to_string()),
            schema: exposed_schema(),
            schema_revision: self.schema_revision(),
            allow_untyped: false,
        })
    }

    fn schema_revision(&self) -> String {
        REVISION.into()
    }

    async fn negotiate(&self, request: &ScanRequest) -> Result<ScanPlan, VdbError> {
        let filters = classify_simple(&request.filters, |field, op| {
            op == BinaryOp::Eq && (field_is(field, "id") || field_is(field, "type"))
        });
        let ordered_prefix = request.order_by.iter().take_while(|order| matches!(&order.expr, Expr::Operand(Operand::Field(path)) if field_is(path, "id"))).count() as u64;
        let complete = filters
            .iter()
            .all(|support| *support == FilterSupport::Exact)
            && ordered_prefix == request.order_by.len() as u64;
        let plan = AcceptedScan {
            filters,
            ordered_prefix,
            limit_applied: complete && request.limit.is_some(),
            offset_applied: complete && request.offset > 0,
            estimated_rows: None,
            token: None,
            schema_revision: self.schema_revision(),
        };
        plan.validate(request)?;
        Ok(ScanPlan::Accepted { plan })
    }

    fn scan(
        &self,
        request: ScanRequest,
        plan: AcceptedScan,
        _bindings: Object,
        cancellation: CancellationToken,
    ) -> EntityStream {
        let directory = self.directory.clone();
        Box::pin(async_stream::try_stream! {
            plan.validate(&request)?;
            let mut entries = tokio::fs::read_dir(&directory).await.map_err(failure)?;
            let mut files = vec![];
            while let Some(entry) = entries.next_entry().await.map_err(failure)? {
                if cancellation.is_cancelled() { Err(failure("scan cancelled"))?; }
                let path = entry.path();
                if path.extension().is_some_and(|extension| extension == "json") && entry.file_type().await.map_err(failure)?.is_file() {
                    let id = entry.file_name().into_string().map_err(|_| failure("JSON file names must be UTF-8"))?;
                    files.push((id, path));
                }
            }
            files.sort_by(|left, right| left.0.cmp(&right.0));
            if plan.ordered_prefix > 0 && request.order_by[0].direction == SortDirection::Desc { files.reverse(); }
            let mut skipped = 0u64;
            let mut emitted = 0u64;
            for (id, path) in files {
                if !request.filters.iter().zip(&plan.filters).all(|(filter, support)| {
                    if *support != FilterSupport::Exact { return true; }
                    let Expr::Binary { left, right, .. } = filter else { return false; };
                    let (Expr::Operand(Operand::Field(field)), Expr::Operand(Operand::Literal(value))) = (&**left, &**right) else { return false; };
                    value == &Value::String(if field_is(field, "id") { id.clone() } else { CLASS.into() })
                }) { continue; }
                if plan.offset_applied && skipped < request.offset { skipped += 1; continue; }
                if plan.limit_applied && request.limit.is_some_and(|limit| emitted >= limit) { break; }
                let bytes = tokio::select! {
                    _ = cancellation.cancelled() => Err(failure("scan cancelled")),
                    bytes = tokio::fs::read(&path) => bytes.map_err(failure),
                }?;
                let content: Value = serde_json::from_slice(&bytes).map_err(|error| failure(format!("{}: {error}", path.display())))?;
                let mut entity = Object::new();
                entity.insert("id", Value::String(id)); entity.insert("type", Value::String(CLASS.into())); entity.insert(CONTENT, content);
                // Projection is a hint; complete entities preserve required attributes.
                yield entity;
                emitted += 1;
            }
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = args
        .next()
        .ok_or("usage: json_dir_vdb DIRECTORY [--stdio | --activation ID]")?;
    let mode = args.next();
    let directory = std::fs::canonicalize(directory)?;
    if !directory.is_dir() {
        return Err("expected a directory".into());
    }
    let database = JsonDirVdb {
        directory: directory.clone(),
    };
    let mut catalog = Catalog::new();
    catalog.upsert_package(semantic_data::bundles::query::package());
    catalog.upsert_package(semantic_vdb::package());
    let export = implementation_descriptor(&catalog, "database")?;
    if mode.as_deref() == Some("--activation") {
        let id = args.next().unwrap_or_else(|| "fx".into());
        let activation = PluginActivation {
            id,
            revision: REVISION.into(),
            enabled: true,
            generation: 1,
            provider: PluginProvider::Stdio {
                program: std::env::current_exe()?.to_string_lossy().into_owned(),
                args: vec![directory.to_string_lossy().into_owned(), "--stdio".into()],
                cwd: None,
                env: BTreeMap::new(),
            },
            configuration: Value::Null,
            configuration_schema: None,
            priority: None,
            exports: vec![portable_export(export)],
            source_bindings: BTreeMap::new(),
        };
        println!("{}", serde_json::to_string_pretty(&activation.to_value())?);
        return Ok(());
    }
    if args.next().is_some() || mode.as_deref().is_some_and(|mode| mode != "--stdio") {
        return Err("unexpected argument".into());
    }
    if mode.as_deref() == Some("--stdio") {
        let plugin = VirtualDatabasePlugin::new(
            PluginManifest {
                id: "example.json-directory".into(),
                revision: REVISION.into(),
                title: "JSON directory".into(),
                exports: vec![export],
                configuration_schema: None,
                source_bindings: BTreeMap::new(),
            },
            move |_| {
                let database = database.clone();
                async move { Ok(database) }
            },
        );
        let implementation = plugin
            .create(PluginInstanceContext {
                scope: "stdio".into(),
                generation: 1,
                configuration: Value::Null,
                cancellation: CancellationToken::new(),
            })
            .await?;
        semantic_rpc::plugin::stdio::serve(
            tokio::io::stdin(),
            tokio::io::stdout(),
            implementation,
            Some(REVISION.into()),
            |_| async { Ok(()) },
        )
        .await?;
    } else {
        let request = ScanRequest::default();
        let ScanPlan::Accepted { plan } = database.negotiate(&request).await? else {
            return Err("scan rejected".into());
        };
        let mut entities = database.scan(request, plan, Object::new(), CancellationToken::new());
        while let Some(entity) = entities.next().await {
            println!("{}", serde_json::to_string(&Value::Object(entity?))?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::query::OrderBy;

    fn equals(field: &str, value: Value) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                field,
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(value))),
        }
    }

    #[tokio::test]
    async fn residual_filters_prevent_pushed_limits_and_offset() {
        let database = JsonDirVdb {
            directory: PathBuf::new(),
        };
        let request = ScanRequest {
            filters: vec![
                equals("type", Value::String(CLASS.into())),
                equals(CONTENT, Value::Null),
            ],
            limit: Some(1),
            offset: 2,
            ..ScanRequest::default()
        };
        let ScanPlan::Accepted { plan } = database.negotiate(&request).await.unwrap() else {
            panic!("accepted")
        };
        assert_eq!(
            plan.filters,
            vec![FilterSupport::Exact, FilterSupport::Unsupported]
        );
        assert!(!plan.limit_applied && !plan.offset_applied);
    }

    #[tokio::test]
    async fn scans_files_with_exact_filter_order_and_limit() {
        let directory = std::env::temp_dir().join(format!(
            "semantic-json-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir(&directory).await.unwrap();
        for (name, content) in [
            ("a.json", "{\"title\":\"Alpha\"}"),
            ("b.json", "[1,2]"),
            ("ignored.txt", "ignored"),
        ] {
            tokio::fs::write(directory.join(name), content)
                .await
                .unwrap();
        }
        let database = JsonDirVdb {
            directory: directory.clone(),
        };
        let request = ScanRequest {
            filters: vec![equals("type", Value::String(CLASS.into()))],
            order_by: vec![OrderBy {
                expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                direction: SortDirection::Desc,
            }],
            limit: Some(1),
            projection: Some(vec!["id".into()]),
            ..ScanRequest::default()
        };
        let ScanPlan::Accepted { plan } = database.negotiate(&request).await.unwrap() else {
            panic!("accepted")
        };
        assert!(plan.limit_applied);
        assert_eq!(plan.ordered_prefix, 1);
        let mut stream = database.scan(request, plan, Object::new(), CancellationToken::new());
        let entity = stream.next().await.unwrap().unwrap();
        assert_eq!(entity.get("id"), Some(&Value::String("b.json".into())));
        assert!(entity.contains_key(CONTENT));
        assert!(stream.next().await.is_none());
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }
}
