use crate::query_ast::{QueryRequest, binary, combine, field, literal, order};
use semantic_data::{
    attr::ATTR_CREATED_AT,
    builtin::DEFAULT_COLLECTION,
    filestore::FILE_CLASS_ID,
    query::{
        BinaryOp, Expr, PatternMatchKind, QueryField as AstQueryField, SelectQuery, SortDirection,
    },
};

use crate::components::StructuredQuery;

pub use crate::components::{sql_ident, sql_string};

const PAGE_SIZE: usize = 1_000;

#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistFilter {
    pub collection: String,
    pub search: String,
    pub images: bool,
    pub audio: bool,
    pub video: bool,
    pub advanced_sql: bool,
    pub sql: String,
    pub expand_to_media: bool,
    pub structured: StructuredQuery,
    pub structured_predicate: Option<Expr>,
}

impl Default for PlaylistFilter {
    fn default() -> Self {
        Self {
            collection: DEFAULT_COLLECTION.to_string(),
            search: String::new(),
            images: true,
            audio: true,
            video: true,
            advanced_sql: false,
            sql: default_raw_query(DEFAULT_COLLECTION),
            expand_to_media: false,
            structured: StructuredQuery::default(),
            structured_predicate: None,
        }
    }
}

pub(crate) fn playlist_query(
    filter: &PlaylistFilter,
    offset: usize,
) -> std::result::Result<QueryRequest, String> {
    if filter.advanced_sql {
        return validate_raw_select(&filter.sql).map(QueryRequest::Sql);
    }
    if filter.collection.trim().is_empty() {
        return Err("Collection is required".to_string());
    }
    let mut kinds = Vec::new();
    if filter.images {
        kinds.push("image/%");
    }
    if filter.audio {
        kinds.push("audio/%");
    }
    if filter.video {
        kinds.push("video/%");
    }
    if kinds.is_empty() {
        return Err("Select at least one media kind".to_string());
    }
    let pattern = |name: &str, value: String, case_insensitive| Expr::PatternMatch {
        kind: PatternMatchKind::Like,
        expr: Box::new(field(Some("e"), name)),
        pattern: Box::new(literal(value)),
        case_insensitive,
        negated: false,
    };
    let mut predicates = vec![
        Expr::InList {
            expr: Box::new(field(Some("e"), "type")),
            list: vec![literal(FILE_CLASS_ID.to_string())],
            negated: false,
        },
        combine(
            BinaryOp::Or,
            kinds
                .into_iter()
                .map(|kind| pattern("mime_type", kind.into(), false)),
        )
        .expect("selected media kinds"),
    ];
    if !filter.search.trim().is_empty() {
        let value = format!("%{}%", filter.search.trim());
        predicates.push(binary(
            BinaryOp::Or,
            pattern("id", value.clone(), true),
            pattern("title", value, true),
        ));
    }
    predicates.extend(filter.structured_predicate.clone());
    let query = SelectQuery::new()
        .with_collection(&filter.collection)
        .with_source_alias("e")
        .with_projection(
            ["id", "type", "title", "mime_type", "media_duration"]
                .into_iter()
                .map(|name| AstQueryField {
                    expr: Box::new(field(Some("e"), name)),
                    alias: Some(name.into()),
                    wildcard: None,
                })
                .collect(),
        )
        .with_predicate(combine(BinaryOp::And, predicates).expect("media predicates"))
        .with_order_by(vec![
            order(Some("e"), ATTR_CREATED_AT, SortDirection::Desc),
            order(Some("e"), "id", SortDirection::Asc),
        ])
        .with_limit(PAGE_SIZE)
        .with_offset(offset);
    Ok(query.into())
}

pub fn page_size() -> usize {
    PAGE_SIZE
}

fn default_raw_query(collection: &str) -> String {
    format!(
        "SELECT * FROM {} WHERE type IN ({}) ORDER BY {} DESC, id ASC LIMIT 100000",
        sql_ident(collection),
        sql_string(FILE_CLASS_ID),
        sql_ident(ATTR_CREATED_AT),
    )
}

fn validate_raw_select(sql: &str) -> std::result::Result<String, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL is required".to_string());
    }
    if !trimmed.to_ascii_lowercase().starts_with("select ") {
        return Err("The playlist accepts read-only SELECT queries only".to_string());
    }
    if trimmed.trim_end_matches(';').contains(';') {
        return Err("Only one SELECT statement is allowed".to_string());
    }
    let normalized = trimmed.to_ascii_lowercase();
    if !normalized.contains(" order by ") || !normalized.contains(" limit ") {
        return Err(
            "Raw playlist SQL must include deterministic ORDER BY and a bounded LIMIT".to_string(),
        );
    }
    Ok(trimmed.trim_end_matches(';').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        FilterGroup, FilterNode, FilterOperator, FilterRule, QueryField, QueryFieldKind,
        compile_structured_predicate,
    };
    use semantic_data::value::Value;

    #[tokio::test]
    async fn media_filters_search_projection_and_pagination_execute_as_ast() {
        use crate::query_ast::tests::{memory_db, row};
        let db = memory_db(
            "query_test",
            [
                row(
                    "a",
                    [
                        ("type", Value::String(FILE_CLASS_ID.into())),
                        ("mime_type", Value::String("image/png".into())),
                        ("title", Value::String("Ada's photo".into())),
                    ],
                ),
                row(
                    "b",
                    [
                        ("type", Value::String(FILE_CLASS_ID.into())),
                        ("mime_type", Value::String("audio/ogg".into())),
                        ("title", Value::String("Ada's audio".into())),
                    ],
                ),
                row(
                    "c",
                    [
                        ("type", Value::String(FILE_CLASS_ID.into())),
                        ("mime_type", Value::String("video/mp4".into())),
                        ("title", Value::String("Ada's video".into())),
                    ],
                ),
                row(
                    "d",
                    [
                        ("type", Value::String(FILE_CLASS_ID.into())),
                        ("mime_type", Value::String("image/png".into())),
                        ("title", Value::String("other".into())),
                    ],
                ),
            ],
        )
        .await;
        let filter = PlaylistFilter {
            collection: "query_test".into(),
            video: false,
            search: "Ada's".into(),
            ..Default::default()
        };
        let QueryRequest::Ast(query) = playlist_query(&filter, 1).unwrap() else {
            panic!("AST")
        };
        let semantic_db_core::QueryResult::Select(rows) = db
            .query(semantic_data::query::QueryInput::from(query))
            .await
            .unwrap()
        else {
            panic!("AST rows")
        };
        assert_eq!(
            rows.iter()
                .map(|row| row.get("id").unwrap().as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
        assert_eq!(
            rows[0].get("mime_type").and_then(Value::as_str),
            Some("audio/ogg")
        );
    }

    #[test]
    fn default_query_is_playable_stable_and_paged() {
        let QueryRequest::Ast(query) = playlist_query(&PlaylistFilter::default(), 2_000).unwrap()
        else {
            panic!("generated AST")
        };
        assert_eq!(query.limit, Some(PAGE_SIZE.into()));
        assert_eq!(query.offset, 2_000usize.into());
        assert_eq!(
            query.order_by,
            vec![
                order(Some("e"), ATTR_CREATED_AT, SortDirection::Desc),
                order(Some("e"), "id", SortDirection::Asc)
            ]
        );
        assert_eq!(query.projection.len(), 5);
        for kind in ["image/%", "audio/%", "video/%"] {
            assert!(crate::query_ast::tests::contains_string(&query, kind));
        }
        assert!(
            PlaylistFilter::default()
                .sql
                .contains("ORDER BY \"semantic:created_at\" DESC, id ASC")
        );
    }

    #[test]
    fn search_and_identifiers_remain_literal_values() {
        let mut filter = PlaylistFilter::default();
        filter.collection = "odd\"collection".to_string();
        filter.search = "O'Brien".to_string();
        let QueryRequest::Ast(query) = playlist_query(&filter, 0).unwrap() else {
            panic!("generated AST")
        };
        assert_eq!(query.collection.as_deref(), Some("odd\"collection"));
        assert!(crate::query_ast::tests::contains_string(
            &query,
            "%O'Brien%"
        ));
    }

    #[test]
    fn raw_mode_rejects_writes_and_multiple_statements() {
        let mut filter = PlaylistFilter::default();
        filter.advanced_sql = true;
        filter.sql = "DELETE FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
        filter.sql = "SELECT * FROM entities; DELETE FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
        filter.sql = "SELECT * FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
    }

    #[test]
    fn structured_predicate_combines_with_mandatory_media_filters() {
        let fields = vec![QueryField {
            name: "rating".to_string(),
            label: "Rating".to_string(),
            description: None,
            kind: QueryFieldKind::SignedInteger,
            deprecated: false,
            choices: Vec::new(),
        }];
        let structured = StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: "rating".to_string(),
                    operator: FilterOperator::GreaterOrEqual,
                    values: vec!["4".to_string()],
                })],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut filter = PlaylistFilter {
            structured: structured.clone(),
            ..Default::default()
        };
        filter.structured_predicate =
            compile_structured_predicate(&structured, &fields, Some("e"), &[]).unwrap();
        let QueryRequest::Ast(query) = playlist_query(&filter, 0).unwrap() else {
            panic!("generated AST")
        };
        let Some(Expr::Binary {
            op: BinaryOp::And,
            right,
            ..
        }) = query.predicate
        else {
            panic!("mandatory media filters combined with user filter")
        };
        assert_eq!(*right, filter.structured_predicate.unwrap());
    }
}
