use crate::query_ast::{binary, combine, field, literal};
use semantic_data::{
    attr::RELATION_CLASS_ID,
    builtin::DEFAULT_COLLECTION,
    query::{BinaryOp, Expr},
};
use semantic_ui_core::UiCatalog;

/// Seed text for the explicit SQL editor. Programmatic requests use listing_predicate.
pub(super) fn listing_sql_predicate(
    collection: &str,
    catalog: Option<&UiCatalog>,
) -> Option<String> {
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

/// Default constraints for generic listings, applied before pagination.
pub(super) fn listing_predicate(collection: &str, catalog: Option<&UiCatalog>) -> Option<Expr> {
    let mut predicates = Vec::new();
    if collection == DEFAULT_COLLECTION {
        predicates.push(binary(
            BinaryOp::NotEq,
            field(None, "type"),
            literal(RELATION_CLASS_ID.to_string()),
        ));
    }
    if let Some(catalog) = catalog {
        let types = catalog.listing_excluded_type_values(collection == DEFAULT_COLLECTION);
        if !types.is_empty() {
            // Untyped records still belong in listings.
            predicates.push(binary(
                BinaryOp::Or,
                Expr::IsNull {
                    expr: Box::new(field(None, "type")),
                    negated: false,
                },
                Expr::InList {
                    expr: Box::new(field(None, "type")),
                    list: types
                        .into_iter()
                        .map(|id| literal(id.to_string()))
                        .collect(),
                    negated: true,
                },
            ));
        }
    }
    combine(BinaryOp::And, predicates)
}
