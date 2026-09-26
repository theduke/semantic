//! Execution of [`PhysicalTextSearch`](crate::PhysicalTextSearch)es: one
//! equality probe of the full-text index per token, combined by id, then
//! point reads of the matching rows.

use semantic_data::query::TextMatchMode;

use super::*;

impl EmbeddedPhysicalDataSource<'_> {
    /// Rows of `search` read through its full-text index, or `None` when
    /// the index is unavailable (the caller then filters a scan).
    pub(super) fn text_search_scan(
        &self,
        search: &crate::PhysicalTextSearch,
    ) -> Result<Option<EmbeddedCollectionScan>, crate::CoreError> {
        let collection = self
            .resolve_collection(&search.source)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
        let Some(index) = self
            .catalog
            .index_by_lid(search.index)
            .filter(|index| index.collection == collection.lid && index.schema.kind.is_full_text())
        else {
            return Ok(None);
        };
        let ids = self
            .text_search_ids(index.lid, &search.tokens, search.mode)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
        Ok(Some(
            self.materialize_ids(collection, ids.into_iter().collect())?,
        ))
    }

    /// Ids of the entries of `index` for all (`All`) or any (`Any`) of
    /// `tokens`, in id order.
    fn text_search_ids(
        &self,
        index: LocalIndexId,
        tokens: &[String],
        mode: TextMatchMode,
    ) -> std::result::Result<BTreeSet<String>, DbError> {
        let mut result: Option<BTreeSet<String>> = None;
        for token in tokens {
            let ids = self
                .reader
                .scan_index_value(index, None, &Value::String(token.clone()))?;
            result = Some(match (result, mode) {
                (None, _) => ids.into_iter().collect(),
                (Some(acc), TextMatchMode::All) => {
                    ids.into_iter().filter(|id| acc.contains(id)).collect()
                }
                (Some(mut acc), TextMatchMode::Any) => {
                    acc.extend(ids);
                    acc
                }
            });
            if mode == TextMatchMode::All && result.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        Ok(result.unwrap_or_default())
    }
}

#[cfg(all(test, feature = "sql"))]
mod tests;
