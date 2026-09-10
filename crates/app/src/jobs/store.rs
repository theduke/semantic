use crate::SemanticDb;
use async_trait::async_trait;
use semantic_data::{Value, builtin::ATTR_ID, jobs::*, query::*};
use semantic_db_core::{
    Batch, BatchOperation, QueryResult,
    catalog::{CollectionKind, IntegrityMode},
};
use semantic_jobs::{JobStore, JobStoreError};
use std::sync::Arc;

/// Adapter over the application's existing database. All writes are awaited;
/// backend errors have settled, although a commit acknowledgement may be lost.
pub struct DbJobStore {
    db: Arc<dyn SemanticDb>,
}
impl DbJobStore {
    pub fn new(db: Arc<dyn SemanticDb>) -> Self {
        Self { db }
    }
}
fn read_error(error: impl std::fmt::Display) -> JobStoreError {
    JobStoreError::definitive(error.to_string())
}
fn write_error(error: impl std::fmt::Display) -> JobStoreError {
    JobStoreError::unknown(error.to_string(), true)
}
fn field(name: &str) -> Expr {
    Expr::Operand(Operand::Field(semantic_data::FieldPath(vec![
        semantic_data::PathSegment::Field(name.to_string()),
    ])))
}
fn literal(value: Value) -> Expr {
    Expr::Operand(Operand::Literal(value))
}
fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

#[async_trait]
impl JobStore for DbJobStore {
    async fn initialize(&self) -> Result<(), JobStoreError> {
        let catalog = self.db.catalog().await.map_err(read_error)?;
        let expected =
            semantic_db_core::normalize_package_definition(&package()).map_err(read_error)?;
        // V1 has no supported older package variants. Fail closed before catalog
        // upsert: the general package API permits replacing newer definitions.
        if let Some(stored) = catalog.package_by_name(PACKAGE_NAME) {
            if stored != &expected {
                return Err(read_error("incompatible stored semantic_jobs package"));
            }
        }
        for (_, applied) in catalog.applied_migrations() {
            if applied.package == PACKAGE_NAME && !expected.migrations.contains(&applied.migration)
            {
                return Err(read_error("incompatible applied semantic_jobs migration"));
            }
        }
        if let Some(collection) = catalog.collection_by_name(COLLECTION) {
            if catalog.package_by_name(PACKAGE_NAME).is_none() {
                return Err(read_error(
                    "semantic_jobs collection already exists without the jobs package",
                ));
            }
            // The current catalog normalizes polymorphic migrations to Schema.
            if !matches!(
                collection.kind,
                CollectionKind::Polymorphic | CollectionKind::Schema
            ) || collection.integrity_mode != IntegrityMode::StrictRegisteredSchema
            {
                return Err(read_error(format!(
                    "incompatible existing semantic_jobs collection: {:?}, {:?}",
                    collection.kind, collection.integrity_mode
                )));
            }
        }
        self.db
            .upsert_package(package())
            .await
            .map_err(write_error)?;
        Ok(())
    }
    async fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        self.db
            .get(COLLECTION.into(), id.0.clone())
            .await
            .map_err(read_error)?
            .map(|record| {
                JobRecord::from_object(JobId(record.id), &record.object).map_err(read_error)
            })
            .transpose()
    }
    async fn put(&self, record: &JobRecord) -> Result<(), JobStoreError> {
        record.validate().map_err(read_error)?;
        self.db
            .execute_batch(Batch {
                operations: vec![BatchOperation::Upsert {
                    collection: COLLECTION.into(),
                    id: record.id.0.clone(),
                    object: record.to_object(),
                }],
            })
            .await
            .map_err(write_error)?;
        Ok(())
    }
    async fn list(&self, query: JobListQuery) -> Result<JobListPage, JobStoreError> {
        if query.limit == 0 {
            return Err(read_error("job page limit must be positive"));
        }
        let mut select = SelectQuery::new().with_collection(COLLECTION);
        select.field_format = FieldFormat::Qualified;
        let mut predicates = Vec::new();
        if !query.statuses.is_empty() {
            predicates.push(Expr::InList {
                expr: Box::new(field(&format!("{PREFIX}status"))),
                list: query
                    .statuses
                    .iter()
                    .map(|s| literal(Value::String(s.as_str().into())))
                    .collect(),
                negated: false,
            });
        }
        if let Some(kind) = query.kind {
            predicates.push(binary(
                BinaryOp::Eq,
                field(&format!("{PREFIX}kind")),
                literal(Value::String(kind.0)),
            ));
        }
        if let Some(cursor) = query.cursor {
            let op = if query.oldest_first {
                BinaryOp::Gt
            } else {
                BinaryOp::Lt
            };
            let created = field(&format!("{PREFIX}created_at"));
            let date = literal(Value::DateTime(cursor.created_at));
            predicates.push(binary(
                BinaryOp::Or,
                binary(op, created.clone(), date.clone()),
                binary(
                    BinaryOp::And,
                    binary(BinaryOp::Eq, created, date),
                    binary(op, field(ATTR_ID), literal(Value::String(cursor.id.0))),
                ),
            ));
        }
        select.predicate = predicates
            .into_iter()
            .reduce(|a, b| binary(BinaryOp::And, a, b));
        let direction = if query.oldest_first {
            SortDirection::Asc
        } else {
            SortDirection::Desc
        };
        select.order_by = vec![
            OrderBy {
                expr: field(&format!("{PREFIX}created_at")),
                direction,
            },
            OrderBy {
                expr: field(ATTR_ID),
                direction,
            },
        ];
        select.limit = Some(Expr::from(query.limit as usize + 1));
        let QueryResult::Select(rows) = self
            .db
            .query_data(select.into())
            .await
            .map_err(read_error)?
        else {
            return Err(read_error("unexpected jobs query result"));
        };
        let mut records = rows
            .into_iter()
            .map(|object| {
                let id = object
                    .get(ATTR_ID)
                    .and_then(Value::as_str)
                    .ok_or_else(|| read_error("job row missing id"))?;
                JobRecord::from_object(JobId(id.into()), &object).map_err(read_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = records.len() > query.limit as usize;
        records.truncate(query.limit as usize);
        let next_cursor = if more {
            records.last().map(|r| JobListCursor {
                created_at: r.created_at,
                id: r.id.clone(),
            })
        } else {
            None
        };
        Ok(JobListPage {
            records,
            next_cursor,
        })
    }
    async fn count(&self) -> Result<u64, JobStoreError> {
        let mut query = SelectQuery::new().with_collection(COLLECTION);
        query.projection = vec![QueryField {
            expr: Box::new(Expr::Aggregate {
                op: AggregateOp::Count,
                distinct: false,
                arg: Box::new(FunctionArg::Wildcard),
            }),
            alias: Some("count".into()),
        }];
        let QueryResult::Select(rows) =
            self.db.query_data(query.into()).await.map_err(read_error)?
        else {
            return Err(read_error("unexpected jobs count result"));
        };
        match rows.first().and_then(|r| r.get("count")) {
            Some(Value::U64(v)) => Ok(*v),
            Some(v) => v
                .as_i64()
                .and_then(|v| u64::try_from(v).ok())
                .ok_or_else(|| read_error("invalid jobs count")),
            None => Err(read_error("missing jobs count")),
        }
    }
    async fn delete_ids(&self, ids: &[JobId]) -> Result<u64, JobStoreError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let outcome = self
            .db
            .execute_batch(Batch {
                operations: vec![BatchOperation::DeleteByIds {
                    collection: COLLECTION.into(),
                    ids: ids.iter().map(|id| id.0.clone()).collect(),
                }],
            })
            .await
            .map_err(write_error)?;
        Ok(outcome.stats.deleted as u64)
    }
}
