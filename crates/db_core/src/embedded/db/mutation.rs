//! Predicate mutations (`UPDATE`/`DELETE ... WHERE`) on the point write path.
//!
//! The rows a predicate can match are located without materializing the
//! collection: through a primary-key equality, through the equality index
//! the query optimizer picks for the equivalent `SELECT` (planned with the
//! statistics of the transaction's snapshot), or else through one streamed
//! scan of the snapshot that keeps only matching rows. Candidates are merged
//! with the rows the transaction has already written.
//!
//! The final match is decided by the dataset executor's own row functions
//! (`apply_update_with_returning_and_prepare`, `apply_delete_with_remaining`)
//! over the candidates in id order. Predicates are therefore evaluated
//! against the stored row exactly as on the dataset path (no local reference
//! resolution), and `LIMIT` keeps the first matching rows by id, so matches,
//! statistics and `RETURNING` projections are unchanged.
use super::compact::TxView;
use super::*;

/// How the rows a mutation predicate can match are located.
#[derive(Debug, PartialEq)]
enum MutationAccess {
    /// The predicate requires one of these primary keys.
    Ids(BTreeSet<String>),
    /// The optimizer reads matches through an equality index.
    Index {
        index: LocalIndexId,
        path: Option<FieldPath>,
        value: Value,
    },
    /// One scan of the collection.
    Scan,
}

impl<S: EntityStorage> EmbeddedDb<S> {
    /// Apply a canonical `UPDATE` to the transaction overlay.
    pub(super) fn compact_update(
        &self,
        view: &mut TxView<'_, S>,
        collection: &str,
        query: &UpdateQuery,
        context: &DefaultExpressionContext,
        recursive_validation: bool,
    ) -> Result<crate::UpdateResult, DbError> {
        let schema = mutation_collection(view.catalog, collection)?;
        let mut entities = self.mutation_candidates(view, schema, query.predicate.as_ref())?;
        let original = entities
            .iter()
            .map(|entity| entity.object.clone())
            .collect::<Vec<_>>();
        let catalog = view.catalog;
        let result =
            crate::apply_update_with_returning_and_prepare(query, &mut entities, |_, object| {
                prepare_row_for_write(catalog, schema, object, context, recursive_validation)
                    .map_err(|err| CoreError::new(err.to_string()))
            })
            .map_err(|err| DbError::InvalidQuery(err.message))?;
        for (entity, original) in entities.into_iter().zip(original) {
            if entity.object != original {
                view.put((collection.to_string(), entity.id), entity.object)?;
            }
        }
        Ok(result)
    }

    /// Apply a canonical `DELETE` to the transaction overlay.
    pub(super) fn compact_delete(
        &self,
        view: &mut TxView<'_, S>,
        collection: &str,
        query: &DeleteQuery,
    ) -> Result<crate::DeleteResult, DbError> {
        let schema = mutation_collection(view.catalog, collection)?;
        let entities = self.mutation_candidates(view, schema, query.predicate.as_ref())?;
        let mut deleted = entities
            .iter()
            .map(|entity| entity.id.clone())
            .collect::<BTreeSet<_>>();
        let (remaining, result) = crate::apply_delete_with_remaining(query, entities);
        for entity in &remaining {
            deleted.remove(&entity.id);
        }
        for id in deleted {
            view.delete((collection.to_string(), id))?;
        }
        Ok(result)
    }

    /// Rows of `collection` that may match `predicate`, ordered by id.
    fn mutation_candidates(
        &self,
        view: &mut TxView<'_, S>,
        collection: &CollectionSchema,
        predicate: Option<&crate::Expr>,
    ) -> Result<Vec<crate::Entity>, DbError> {
        let access = match view.reader().snapshot() {
            Some(snapshot) => {
                self.mutation_access(view.catalog, snapshot, collection, predicate)?
            }
            None => self.mutation_access(
                view.catalog,
                &*self.storage.snapshot()?,
                collection,
                predicate,
            )?,
        };
        let rows = match access {
            MutationAccess::Ids(ids) => view.rows_by_ids(&collection.name, ids)?,
            MutationAccess::Index { index, path, value } => {
                let ids = view.index_ids(index, path.as_ref(), &value)?;
                view.rows_by_ids(&collection.name, ids)?
            }
            MutationAccess::Scan => view.scan_matching(collection, predicate)?,
        };
        Ok(rows
            .into_iter()
            .map(|(id, object)| crate::Entity {
                id,
                collection: collection.name.clone(),
                object,
            })
            .collect())
    }

    /// Choose how to locate the rows `predicate` can match.
    fn mutation_access(
        &self,
        catalog: &Catalog,
        reader: &dyn EntityReadSnapshot,
        collection: &CollectionSchema,
        predicate: Option<&crate::Expr>,
    ) -> Result<MutationAccess, DbError> {
        let Some(predicate) = predicate else {
            return Ok(MutationAccess::Scan);
        };
        if let Some(ids) = primary_key_ids(collection, predicate) {
            return Ok(MutationAccess::Ids(ids));
        }
        let select = SelectQuery::new()
            .with_collection(collection.name.clone())
            .with_predicate(predicate.clone());
        let stats = super::reader::stats_for_query(catalog, reader, &select, collection)?;
        let physical = crate::Optimizer::core()
            .optimize_query(
                &select,
                Some(collection.name.clone()),
                Some(&stats),
                &self.query_context(),
            )
            .physical;
        let Some((field, value)) = find_index_lookup(&physical) else {
            return Ok(MutationAccess::Scan);
        };
        let Some(field_path) = lookup_field_path(collection, field) else {
            return Ok(MutationAccess::Scan);
        };
        if !matches!(field_path.segments().first(), Some(PathSegment::Field(_))) {
            return Ok(MutationAccess::Scan);
        }
        Ok(
            match equality_lookup_index(catalog, collection.lid, &field_path) {
                Some((index, path)) => MutationAccess::Index {
                    index: index.lid,
                    path,
                    value: value.clone(),
                },
                None => MutationAccess::Scan,
            },
        )
    }
}

fn mutation_collection<'c>(
    catalog: &'c Catalog,
    name: &str,
) -> Result<&'c CollectionSchema, DbError> {
    catalog
        .collection_by_name(name)
        .ok_or_else(|| DbError::UnknownCollectionByName {
            name: name.to_string(),
        })
}

/// Primary keys a row must have to match `predicate`: from a top-level
/// conjunct `id = 'literal'` or `id IN ('a', 'b')`.
///
/// Stored rows hold their key in the canonical id field (writes validate
/// this), so rows with other keys cannot match.
fn primary_key_ids(
    collection: &CollectionSchema,
    predicate: &crate::Expr,
) -> Option<BTreeSet<String>> {
    use crate::{Expr, Operand};

    let id_field = collection.canonical_field_name("id");
    let is_id = |expr: &Expr| {
        matches!(
            expr,
            Expr::Operand(Operand::Field(path))
                if matches!(path.segments(), [PathSegment::Field(field)] if field == id_field)
        )
    };
    let string_literal = |expr: &Expr| match expr {
        Expr::Operand(Operand::Literal(Value::String(id))) => Some(id.clone()),
        _ => None,
    };
    match predicate {
        Expr::Binary {
            op: semantic_data::query::BinaryOp::And,
            left,
            right,
        } => primary_key_ids(collection, left).or_else(|| primary_key_ids(collection, right)),
        Expr::Binary {
            op: semantic_data::query::BinaryOp::Eq,
            left,
            right,
        } => {
            let id = if is_id(left) {
                string_literal(right)
            } else if is_id(right) {
                string_literal(left)
            } else {
                None
            }?;
            Some(BTreeSet::from([id]))
        }
        Expr::InList {
            expr,
            list,
            negated: false,
        } if is_id(expr) => list.iter().map(string_literal).collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
