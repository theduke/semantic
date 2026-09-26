//! Trivial reference model of the database: one `BTreeMap` of rows per
//! collection, with predicates and assignments evaluated by the core
//! expression evaluator.
//!
//! The model predicts the outcome of plain writes (including duplicate
//! creates, failing assignments and unique violations) independently of
//! the engines, so a bug shared by all engines still shows up as a
//! difference to the model.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use semantic_data::query::{BatchOperation, DeleteQuery, Expr, UpdateQuery};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{Expr as CoreExpr, evaluate_expr, evaluate_filter_expr, set_value_at_path};

use super::schema::Schema;

/// Outcome kinds the model predicts, named like the engines' errors (see
/// [`super::error_kind`]).
pub(super) const EXISTS: &str = "EntityExists";
pub(super) const UNIQUE: &str = "UniqueViolation";
pub(super) const ASSIGNMENT: &str = "InvalidQuery";

pub(super) type Rows = BTreeMap<String, Object>;

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Model {
    pub collections: BTreeMap<String, Rows>,
}

impl Model {
    pub fn rows(&self, collection: &str) -> impl Iterator<Item = &Object> {
        self.collections
            .get(collection)
            .into_iter()
            .flat_map(|rows| rows.values())
    }

    fn rows_mut(&mut self, collection: &str) -> &mut Rows {
        self.collections.entry(collection.to_string()).or_default()
    }

    pub fn contains(&self, collection: &str, id: &str) -> bool {
        self.collections
            .get(collection)
            .is_some_and(|rows| rows.contains_key(id))
    }

    pub fn ids(&self, collection: &str) -> Vec<String> {
        self.collections
            .get(collection)
            .map(|rows| rows.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Rows of `collection` matching `predicate`, sorted by `order` (each
    /// key ascending) and then by id.
    pub fn select(
        &self,
        collection: &str,
        predicate: Option<&Expr>,
        order: &[FieldPath],
    ) -> Vec<Object> {
        let predicate = predicate.cloned().map(CoreExpr::from);
        let mut rows = self
            .rows(collection)
            .filter(|row| {
                predicate
                    .as_ref()
                    .is_none_or(|predicate| evaluate_filter_expr(*row, predicate))
            })
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            order
                .iter()
                .map(|path| a.get_path(path).cmp(&b.get_path(path)))
                .find(|ordering| *ordering != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.get("id").cmp(&b.get("id")))
        });
        rows
    }

    /// Apply the by-id operations of a batch atomically.
    pub fn apply_batch(
        &mut self,
        operations: &[BatchOperation],
        schema: &Schema,
    ) -> Result<(), &'static str> {
        let mut next = self.clone();
        for operation in operations {
            match operation {
                BatchOperation::Create {
                    collection,
                    id,
                    object,
                } => {
                    if next.contains(collection, id) {
                        return Err(EXISTS);
                    }
                    next.rows_mut(collection).insert(id.clone(), object.clone());
                }
                BatchOperation::Upsert {
                    collection,
                    id,
                    object,
                } => {
                    next.rows_mut(collection).insert(id.clone(), object.clone());
                }
                BatchOperation::DeleteById { collection, id } => {
                    next.rows_mut(collection).remove(id);
                }
                BatchOperation::DeleteByIds { collection, ids } => {
                    for id in ids {
                        next.rows_mut(collection).remove(id);
                    }
                }
                BatchOperation::Update { .. } | BatchOperation::Delete { .. } => {
                    unreachable!("the differential test issues predicate writes on their own")
                }
            }
        }
        next.check_unique(schema)?;
        *self = next;
        Ok(())
    }

    /// Apply an `UPDATE`; returns `(matched, affected)`.
    pub fn update_where(
        &mut self,
        query: &UpdateQuery,
        schema: &Schema,
    ) -> Result<(usize, usize), &'static str> {
        let collection = query.collection.as_deref().unwrap();
        let predicate = query.predicate.clone().map(CoreExpr::from);
        let assignments = query
            .assignments
            .iter()
            .map(|assignment| (&assignment.path, CoreExpr::from(assignment.value.clone())))
            .collect::<Vec<_>>();
        let mut next = self.clone();
        let (mut matched, mut affected) = (0, 0);
        for row in next.rows_mut(collection).values_mut() {
            if !predicate
                .as_ref()
                .is_none_or(|predicate| evaluate_filter_expr(row, predicate))
            {
                continue;
            }
            matched += 1;
            let mut updated = row.clone();
            for (path, value) in &assignments {
                // Assignments observe the row before the update.
                let value = evaluate_expr(row, value).ok_or(ASSIGNMENT)?;
                set_value_at_path(&mut updated, path, value).map_err(|_| ASSIGNMENT)?;
            }
            if updated != *row {
                affected += 1;
                *row = updated;
            }
        }
        next.check_unique(schema)?;
        *self = next;
        Ok((matched, affected))
    }

    /// Apply a `DELETE`; returns the number of deleted rows.
    pub fn delete_where(&mut self, query: &DeleteQuery) -> usize {
        let collection = query.collection.as_deref().unwrap();
        let predicate = query.predicate.clone().map(CoreExpr::from);
        let rows = self.rows_mut(collection);
        let before = rows.len();
        rows.retain(|_, row| {
            !predicate
                .as_ref()
                .is_none_or(|predicate| evaluate_filter_expr(row, predicate))
        });
        before - rows.len()
    }

    /// No two rows share a value of a unique field (rows without the field
    /// are not indexed).
    pub fn check_unique(&self, schema: &Schema) -> Result<(), &'static str> {
        for (collection, rows) in &self.collections {
            for field in schema.unique_fields(collection) {
                let mut seen = Vec::<&Value>::new();
                for row in rows.values() {
                    let Some(value) = row.get(&field) else {
                        continue;
                    };
                    if seen
                        .iter()
                        .any(|other| (*other).cmp(value) == Ordering::Equal)
                    {
                        return Err(UNIQUE);
                    }
                    seen.push(value);
                }
            }
        }
        Ok(())
    }
}

trait GetPath {
    fn get_path(&self, path: &FieldPath) -> Option<&Value>;
}

impl GetPath for Object {
    fn get_path(&self, path: &FieldPath) -> Option<&Value> {
        let [semantic_data::value::PathSegment::Field(name)] = path.segments() else {
            unreachable!("order keys are top-level fields")
        };
        self.get(name)
    }
}
