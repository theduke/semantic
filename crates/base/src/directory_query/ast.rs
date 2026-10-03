//! Directly constructed counterparts to the compatible SQL directory helpers.

use semantic_data::{
    attr::{ATTR_CREATED_AT, ATTR_RELATION_RELATION, ATTR_RELATION_TO, ATTR_TITLE},
    builtin::DEFAULT_COLLECTION,
    bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
        DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
    },
    query::{
        BinaryOp, Expr, FieldFormat, JoinCondition, JoinQuery, JoinSource, JoinType, Operand,
        OrderBy, QueryField, SelectQuery, SortDirection, UnaryOp,
    },
    value::{FieldPath, Value},
};

use super::{DirectoryChildFilter, DirectoryQueryPage, DirectorySort};

fn field(alias: &str, name: &str) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields([alias, name])))
}

fn literal(value: &str) -> Expr {
    Expr::Operand(Operand::Literal(Value::String(value.into())))
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn eq(alias: &str, name: &str, value: &str) -> Expr {
    binary(BinaryOp::Eq, field(alias, name), literal(value))
}

fn all(predicates: impl IntoIterator<Item = Expr>) -> Expr {
    predicates
        .into_iter()
        .reduce(|a, b| binary(BinaryOp::And, a, b))
        .unwrap()
}

fn class(alias: &str, name: &str) -> Expr {
    Expr::InList {
        expr: Box::new(field(alias, "type")),
        list: vec![literal(name)],
        negated: false,
    }
}

fn projection(alias: &str, name: &str, output: &str) -> QueryField {
    QueryField {
        expr: Box::new(field(alias, name)),
        alias: Some(output.into()),
        wildcard: None,
    }
}

fn wildcard(alias: &str) -> QueryField {
    QueryField {
        expr: Box::new(Expr::Operand(Operand::Literal(Value::Null))),
        alias: None,
        wildcard: Some(FieldPath::from_fields([alias])),
    }
}

fn order(alias: &str, name: &str, direction: SortDirection) -> OrderBy {
    OrderBy {
        expr: field(alias, name),
        direction,
    }
}

fn select(alias: &str) -> SelectQuery {
    SelectQuery::new()
        .with_collection(DEFAULT_COLLECTION)
        .with_source_alias(alias)
        .with_field_format(FieldFormat::Qualified)
}

fn paged(mut query: SelectQuery, page: DirectoryQueryPage) -> SelectQuery {
    if let DirectoryQueryPage::Page { limit, offset } = page {
        query = query.with_limit(limit).with_offset(offset);
    }
    query
}

fn node_predicate() -> Expr {
    all([
        class("n", DIRECTORY_NODE_CLASS_ID),
        eq("n", ATTR_RELATION_RELATION, DIRECTORY_NODE_RELATION_ID),
    ])
}

fn child_join() -> JoinQuery {
    JoinQuery {
        source: JoinSource {
            collection: Some(DEFAULT_COLLECTION.into()),
            class: None,
        },
        alias: Some("child".into()),
        join_type: JoinType::Inner,
        condition: JoinCondition::OnExpr(binary(
            BinaryOp::Eq,
            field("n", ATTR_RELATION_TO),
            field("child", "id"),
        )),
        predicate: None,
    }
}

/// List directory entities with the same qualified fields and ordering as the SQL helper.
pub fn directories_query_ast(page: DirectoryQueryPage) -> SelectQuery {
    paged(
        select("d")
            .with_projection(vec![wildcard("d")])
            .with_predicate(class("d", DIRECTORY_CLASS_ID))
            .with_order_by(vec![
                order("d", "title", SortDirection::Asc),
                order("d", "id", SortDirection::Asc),
            ]),
        page,
    )
}

/// Find root directories with an exact title, excluding directories linked below a parent.
pub fn root_directories_named_query_ast(title: &str, max_results: usize) -> SelectQuery {
    let excluded = select("n")
        .with_projection(vec![projection("n", ATTR_RELATION_TO, ATTR_RELATION_TO)])
        .with_predicate(node_predicate());
    directories_query_ast(DirectoryQueryPage::All)
        .with_predicate(all([
            class("d", DIRECTORY_CLASS_ID),
            binary(
                BinaryOp::Or,
                eq("d", "title", title),
                eq("d", ATTR_TITLE, title),
            ),
            Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(binary(
                    BinaryOp::In,
                    field("d", "id"),
                    Expr::Subquery(Box::new(excluded)),
                )),
            },
        ]))
        .with_limit(max_results)
}

/// Find one directory by its identifier.
pub fn directory_by_id_query_ast(id: &str) -> SelectQuery {
    select("d")
        .with_projection(vec![wildcard("d")])
        .with_predicate(all([eq("d", "id", id), class("d", DIRECTORY_CLASS_ID)]))
        .with_order_by(vec![order("d", "id", SortDirection::Asc)])
        .with_limit(1usize)
}

/// List directory links with stable directory_from, directory_to and directory_order aliases.
pub fn directory_links_query_ast(page: DirectoryQueryPage) -> SelectQuery {
    paged(
        select("n")
            .with_projection(vec![
                projection("n", "id", "id"),
                projection("n", ATTR_DIRECTORY_NODE_FROM, "directory_from"),
                projection("n", ATTR_RELATION_TO, "directory_to"),
                projection("n", ATTR_DIRECTORY_NODE_ORDER, "directory_order"),
            ])
            .with_predicate(node_predicate())
            .with_order_by(vec![
                order("n", ATTR_DIRECTORY_NODE_ORDER, SortDirection::Asc),
                order("n", "id", SortDirection::Asc),
            ]),
        page,
    )
}

/// List linked children alongside their parent and order metadata.
pub fn directory_tree_items_query_ast(page: DirectoryQueryPage) -> SelectQuery {
    paged(
        select("n")
            .with_joins(vec![child_join()])
            .with_projection(vec![
                wildcard("child"),
                projection("n", ATTR_DIRECTORY_NODE_FROM, "directory_from"),
                projection("n", ATTR_RELATION_TO, "directory_to"),
                projection("n", ATTR_DIRECTORY_NODE_ORDER, "directory_order"),
            ])
            .with_predicate(node_predicate())
            .with_order_by(vec![
                order("n", ATTR_DIRECTORY_NODE_ORDER, SortDirection::Asc),
                order("child", "title", SortDirection::Asc),
                order("child", "id", SortDirection::Asc),
            ]),
        page,
    )
}

/// List and page a directory’s children, applying its class filter before pagination.
pub fn directory_children_query_ast(
    parent_id: &str,
    filter: DirectoryChildFilter,
    sort: DirectorySort,
    page: DirectoryQueryPage,
) -> SelectQuery {
    let mut predicates = vec![
        node_predicate(),
        eq("n", ATTR_DIRECTORY_NODE_FROM, parent_id),
    ];
    if filter == DirectoryChildFilter::Directories {
        predicates.push(class("child", DIRECTORY_CLASS_ID));
    }
    use SortDirection::{Asc, Desc};
    let orders = match sort {
        DirectorySort::Order => vec![
            order("n", ATTR_DIRECTORY_NODE_ORDER, Asc),
            order("child", "title", Asc),
            order("child", "id", Asc),
        ],
        DirectorySort::TitleAsc => vec![order("child", "title", Asc), order("child", "id", Asc)],
        DirectorySort::TitleDesc => vec![order("child", "title", Desc), order("child", "id", Asc)],
        DirectorySort::TypeAsc => vec![
            order("child", "type", Asc),
            order("child", "title", Asc),
            order("child", "id", Asc),
        ],
        DirectorySort::CreatedAtDesc => vec![
            order("child", ATTR_CREATED_AT, Desc),
            order("child", "id", Asc),
        ],
        DirectorySort::UpdatedAtDesc => vec![
            order("child", "updated_at", Desc),
            order("child", "title", Asc),
            order("child", "id", Asc),
        ],
        DirectorySort::IdAsc => vec![order("child", "id", Asc)],
    };
    paged(
        select("n")
            .with_joins(vec![child_join()])
            .with_projection(vec![
                wildcard("child"),
                projection("n", ATTR_DIRECTORY_NODE_ORDER, "directory_order"),
            ])
            .with_predicate(all(predicates))
            .with_order_by(orders),
        page,
    )
}

/// Find the first parent link ordered by its directory order and identifier.
pub fn directory_parent_query_ast(child_id: &str) -> SelectQuery {
    select("n")
        .with_projection(vec![
            projection("n", "id", "id"),
            projection("n", ATTR_DIRECTORY_NODE_FROM, "directory_from"),
            projection("n", ATTR_DIRECTORY_NODE_ORDER, "directory_order"),
        ])
        .with_predicate(all([node_predicate(), eq("n", ATTR_RELATION_TO, child_id)]))
        .with_order_by(vec![
            order("n", ATTR_DIRECTORY_NODE_ORDER, SortDirection::Asc),
            order("n", "id", SortDirection::Asc),
        ])
        .with_limit(1usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::value::Object;
    use semantic_db_core::{Db, QueryResult};
    use semantic_db_kv::{MemoryBackend, open_memory};

    #[tokio::test]
    async fn directory_builders_execute_ast_queries() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(crate::package()).await.unwrap();
        for (id, title, ty) in [
            ("root'1", "Ada's files", DIRECTORY_CLASS_ID),
            ("child-dir", "Ada's files", DIRECTORY_CLASS_ID),
            ("file", "A file", "semantic:base:person"),
        ] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert("type", Value::String(ty.into()));
            object.insert("title", Value::String(title.into()));
            db.insert(DEFAULT_COLLECTION, id, object).await.unwrap();
        }
        for (id, child, order) in [("node-a", "child-dir", 2), ("node-b", "file", 1)] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.into()));
            object.insert(
                ATTR_RELATION_RELATION,
                Value::String(DIRECTORY_NODE_RELATION_ID.into()),
            );
            object.insert(ATTR_DIRECTORY_NODE_FROM, Value::String("root'1".into()));
            object.insert(ATTR_RELATION_TO, Value::String(child.into()));
            object.insert(ATTR_DIRECTORY_NODE_ORDER, Value::U64(order));
            db.insert(DEFAULT_COLLECTION, id, object).await.unwrap();
        }
        let cases: Vec<(SelectQuery, Vec<&str>)> = vec![
            (
                directories_query_ast(DirectoryQueryPage::All),
                vec!["child-dir", "root'1"],
            ),
            (
                root_directories_named_query_ast("Ada's files", 2),
                vec!["root'1"],
            ),
            (directory_by_id_query_ast("root'1"), vec!["root'1"]),
            (
                directory_links_query_ast(DirectoryQueryPage::All),
                vec!["node-b", "node-a"],
            ),
            (
                directory_tree_items_query_ast(DirectoryQueryPage::All),
                vec!["file", "child-dir"],
            ),
            (directory_parent_query_ast("child-dir"), vec!["node-a"]),
            (
                directory_children_query_ast(
                    "root'1",
                    DirectoryChildFilter::All,
                    DirectorySort::Order,
                    DirectoryQueryPage::All,
                ),
                vec!["file", "child-dir"],
            ),
            (
                directory_children_query_ast(
                    "root'1",
                    DirectoryChildFilter::Directories,
                    DirectorySort::Order,
                    DirectoryQueryPage::All,
                ),
                vec!["child-dir"],
            ),
            (
                directory_children_query_ast(
                    "root'1",
                    DirectoryChildFilter::All,
                    DirectorySort::TitleDesc,
                    DirectoryQueryPage::new(1, 1),
                ),
                vec!["file"],
            ),
        ];
        for (query, expected) in cases {
            let QueryResult::Select(rows) = db.query(query).await.unwrap() else {
                panic!("expected rows")
            };
            let ids = rows
                .iter()
                .map(|row| row.get("id").unwrap().as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(ids, expected);
        }
    }
}
