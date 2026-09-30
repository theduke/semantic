use semantic_data::{attr::RELATION_CLASS_ID, builtin::DEFAULT_COLLECTION};
use semantic_ui_core::UiCatalog;

/// Default constraints for generic listings, applied before pagination.
pub(super) fn listing_predicate(collection: &str, catalog: Option<&UiCatalog>) -> Option<String> {
    let mut predicates = Vec::new();
    if collection == DEFAULT_COLLECTION {
        predicates.push(format!("type != '{RELATION_CLASS_ID}'"));
    }
    if let Some(catalog) = catalog {
        let types = catalog.listing_excluded_type_values(collection == DEFAULT_COLLECTION);
        if !types.is_empty() {
            let literals = types
                .into_iter()
                .map(|id| format!("'{}'", id.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ");
            // Untyped records still belong in listings.
            predicates.push(format!("(type IS NULL OR type NOT IN ({literals}))"));
        }
    }
    (!predicates.is_empty()).then(|| predicates.join(" AND "))
}
