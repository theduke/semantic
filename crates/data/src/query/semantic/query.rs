use super::*;

ast_enum!(query::AggregateOp, "semantic:query:AggregateOp", {
    Count => "count",
    Sum => "sum",
    Avg => "avg",
    Min => "min",
    Max => "max",
});

ast_record!(query::Assignment, "semantic:query:Assignment", {
    path: value::FieldPath => "path" [required],
    value: query::Expr => "value" [required],
});

ast_enum!(query::BinaryOp, "semantic:query:BinaryOp", {
    Add => "add",
    Sub => "sub",
    Mul => "mul",
    Div => "div",
    Mod => "mod",
    Concat => "concat",
    And => "and",
    Or => "or",
    Eq => "eq",
    NotEq => "not_eq",
    Lt => "lt",
    Lte => "lte",
    Gt => "gt",
    Gte => "gte",
    In => "in",
});

ast_record!(query::DdlBatch, "semantic:query:DdlBatch", {
    operations: Vec<query::DdlOperation> => "operations" [default Vec::new()],
});

ast_enum!(query::DdlCollectionKind, "semantic:query:DdlCollectionKind", {
    Untyped => "untyped",
    Schema => "schema",
    Polymorphic => "polymorphic",
});

ast_variant!(query::DdlOperation, "semantic:query:DdlOperation", {
    UpsertAttribute => "upsert_attribute" {
        attribute: schema::AttributeType => "attribute" [required],
    },
    DeleteAttribute => "delete_attribute" {
        id: String => "id" [required],
    },
    UpsertTypeDef => "upsert_type_def" {
        type_def: schema::TypeDef => "type_def" [required],
    },
    DeleteTypeDef => "delete_type_def" {
        name: String => "name" [required],
    },
    UpsertRecordType => "upsert_record_type" {
        id: String => "id" [required],
        name: String => "name" [required],
        record: schema::RecordType => "record" [required],
    },
    DeleteRecordType => "delete_record_type" {
        id: String => "id" [required],
    },
    UpsertClass => "upsert_class" {
        class: schema::ClassType => "class" [required],
    },
    DeleteClass => "delete_class" {
        id: String => "id" [required],
    },
    UpsertCollection => "upsert_collection" {
        name: String => "name" [required],
        kind: query::DdlCollectionKind => "kind" [required],
        integrity_mode: query::IntegrityMode => "integrity_mode" [required],
    },
    DeleteCollection => "delete_collection" {
        name: String => "name" [required],
    },
    UpsertIndex => "upsert_index" {
        name: String => "name" [required],
        collection: String => "collection" [required],
        field: String => "field" [required],
        unique: bool => "unique" [required],
        kind: schema::IndexKind => "kind" [default schema::IndexKind::Equality],
        extra_fields: Vec<String> => "extra_fields" [default Vec::new()],
        predicate: Option<query::Expr> => "predicate" [default None],
        analyzer: query::TextAnalyzer => "analyzer" [default query::TextAnalyzer::default()],
    },
    DeleteIndex => "delete_index" {
        name: String => "name" [required],
        collection: String => "collection" [required],
    },
    UpsertRelationship => "upsert_relationship" {
        relationship: schema::RelationType => "relationship" [required],
    },
    DeleteRelationship => "delete_relationship" {
        id: String => "id" [required],
    },
    SetAutoIndex => "set_auto_index" {
        enabled: bool => "enabled" [required],
    },
});

ast_record!(query::DdlQuery, "semantic:query:DdlQuery", {
    batch: query::DdlBatch => "batch" [required],
});

ast_record!(query::DeleteQuery, "semantic:query:DeleteQuery", {
    collection: Option<String> => "collection" [default None],
    predicate: Option<query::Expr> => "predicate" [default None],
    limit: Option<query::Expr> => "limit" [default None],
    returning: Vec<query::QueryField> => "returning" [default Vec::new()],
    field_format: query::FieldFormat => "field_format" [default query::FieldFormat::default()],
});

ast_variant!(query::Expr, "semantic:query:Expr", {
    Operand => "operand" (inner: query::Operand),
    ProjectionRef => "projection_ref" (inner: Box<query::Expr>),
    Unary => "unary" {
        op: query::UnaryOp => "op" [required],
        expr: Box<query::Expr> => "expr" [required],
    },
    Binary => "binary" {
        op: query::BinaryOp => "op" [required],
        left: Box<query::Expr> => "left" [required],
        right: Box<query::Expr> => "right" [required],
    },
    IfElse => "if_else" {
        cond: Box<query::Expr> => "cond" [required],
        then_expr: Box<query::Expr> => "then_expr" [required],
        else_expr: Box<query::Expr> => "else_expr" [required],
    },
    Coalesce => "coalesce" (inner: Vec<query::Expr>),
    Function => "function" {
        name: String => "name" [required],
        args: Vec<query::FunctionArg<query::Expr>> => "args" [required],
    },
    Aggregate => "aggregate" {
        op: query::AggregateOp => "op" [required],
        distinct: bool => "distinct" [default false],
        arg: Box<query::FunctionArg<query::Expr>> => "arg" [required],
    },
    InList => "in_list" {
        expr: Box<query::Expr> => "expr" [required],
        list: Vec<query::Expr> => "list" [required],
        negated: bool => "negated" [required],
    },
    Subquery => "subquery" (inner: Box<query::SelectQuery>),
    Between => "between" {
        expr: Box<query::Expr> => "expr" [required],
        low: Box<query::Expr> => "low" [required],
        high: Box<query::Expr> => "high" [required],
        negated: bool => "negated" [required],
    },
    PatternMatch => "pattern_match" {
        kind: query::PatternMatchKind => "kind" [required],
        expr: Box<query::Expr> => "expr" [required],
        pattern: Box<query::Expr> => "pattern" [required],
        case_insensitive: bool => "case_insensitive" [required],
        negated: bool => "negated" [required],
    },
    RegexMatch => "regex_match" {
        expr: Box<query::Expr> => "expr" [required],
        pattern: Box<query::Expr> => "pattern" [required],
        case_insensitive: bool => "case_insensitive" [required],
        negated: bool => "negated" [required],
    },
    TextMatch => "text_match" {
        exprs: Vec<query::Expr> => "exprs" [required],
        query: Box<query::Expr> => "query" [required],
        mode: query::TextMatchMode => "mode" [default query::TextMatchMode::All],
        analyzer: query::TextAnalyzer => "analyzer" [default query::TextAnalyzer::default()],
    },
    IsNull => "is_null" {
        expr: Box<query::Expr> => "expr" [required],
        negated: bool => "negated" [required],
    },
    Exists => "exists" {
        query: Box<query::SelectQuery> => "query" [required],
        negated: bool => "negated" [required],
    },
    RelationExists => "relation_exists" {
        relation: Box<query::Expr> => "relation" [required],
        source: Box<query::Expr> => "source" [required],
        target: Box<query::Expr> => "target" [required],
        transitive: bool => "transitive" [required],
        max_depth: Option<Box<query::Expr>> => "max_depth" [default None],
    },
});

ast_enum!(query::FieldFormat, "semantic:query:FieldFormat", {
    Qualified => "qualified",
    Underscore => "underscore",
    Plain => "plain",
});

ast_variant!(query::FunctionArg<query::Expr>, "semantic:query:FunctionArg", {
    Expr => "expr" (inner: query::Expr),
    Wildcard => "wildcard",
});

ast_record!(query::InsertQuery, "semantic:query:InsertQuery", {
    collection: Option<String> => "collection" [default None],
    columns: Vec<String> => "columns" [default Vec::new()],
    source: query::InsertSource => "source" [required],
    returning: Vec<query::QueryField> => "returning" [default Vec::new()],
    field_format: query::FieldFormat => "field_format" [default query::FieldFormat::default()],
});

ast_variant!(query::InsertSource, "semantic:query:InsertSource", {
    Objects => "objects" (inner: Vec<value::Object>),
    Values => "values" (inner: Vec<Vec<query::Expr>>),
    Select => "select" (inner: query::SelectQuery),
});

ast_enum!(query::IntegrityMode, "semantic:query:IntegrityMode", {
    Permissive => "permissive",
    StrictRegisteredSchema => "strict_registered_schema",
});

ast_variant!(query::JoinCondition, "semantic:query:JoinCondition", {
    OnExpr => "on_expr" (inner: query::Expr),
    UsingFields => "using_fields" {
        left: value::FieldPath => "left" [required],
        right: value::FieldPath => "right" [required],
    },
});

ast_record!(query::JoinQuery, "semantic:query:JoinQuery", {
    source: query::JoinSource => "source" [required],
    alias: Option<String> => "alias" [default None],
    join_type: query::JoinType => "join_type" [required],
    condition: query::JoinCondition => "condition" [required],
    predicate: Option<query::Expr> => "predicate" [default None],
});

ast_record!(query::JoinSource, "semantic:query:JoinSource", {
    collection: Option<String> => "collection" [default None],
    class: Option<String> => "class" [default None],
});

ast_enum!(query::JoinType, "semantic:query:JoinType", {
    Inner => "inner",
    Left => "left",
    Right => "right",
    Full => "full",
});

ast_variant!(query::Operand, "semantic:query:Operand", {
    Field => "field" (inner: value::FieldPath),
    Literal => "literal" (inner: value::Value),
    Parameter => "parameter" (inner: String),
});

ast_record!(query::OrderBy, "semantic:query:OrderBy", {
    expr: query::Expr => "expr" [required],
    direction: query::SortDirection => "direction" [required],
});

ast_enum!(query::PatternMatchKind, "semantic:query:PatternMatchKind", {
    Like => "like",
    SimilarTo => "similar_to",
});

ast_variant!(query::Query, "semantic:query:Query", {
    Select => "select" (inner: query::SelectQuery),
    Insert => "insert" (inner: query::InsertQuery),
    Update => "update" (inner: query::UpdateQuery),
    Delete => "delete" (inner: query::DeleteQuery),
    Ddl => "ddl" (inner: query::DdlQuery),
});

ast_record!(query::QueryField, "semantic:query:QueryField", {
    expr: Box<query::Expr> => "expr" [required],
    alias: Option<String> => "alias" [default None],
    wildcard: Option<value::FieldPath> => "wildcard" [default None],
});

ast_variant!(query::QueryInput, "semantic:query:QueryInput", {
    Ast => "ast" (inner: query::Query),
    AstWithParams => "ast_with_params" {
        query: query::Query => "query" [required],
        params: BTreeMap<String, value::Value> => "params" [default BTreeMap::new()],
    },
    Text => "text" {
        format: query::TextQueryFormat => "format" [required],
        query: String => "query" [required],
        params: BTreeMap<String, value::Value> => "params" [default BTreeMap::new()],
    },
});

ast_record!(query::SelectQuery, "semantic:query:SelectQuery", {
    collection: Option<String> => "collection" [default None],
    source_alias: Option<String> => "source_alias" [default None],
    joins: Vec<query::JoinQuery> => "joins" [default Vec::new()],
    predicate: Option<query::Expr> => "predicate" [default None],
    projection: Vec<query::QueryField> => "projection" [default Vec::new()],
    distinct: bool => "distinct" [default false],
    group_by: Vec<query::Expr> => "group_by" [default Vec::new()],
    having: Option<query::Expr> => "having" [default None],
    order_by: Vec<query::OrderBy> => "order_by" [default Vec::new()],
    offset: query::Expr => "offset" [default query::Expr::from(0usize)],
    limit: Option<query::Expr> => "limit" [default None],
    field_format: query::FieldFormat => "field_format" [default query::FieldFormat::default()],
});

ast_enum!(query::SortDirection, "semantic:query:SortDirection", {
    Asc => "asc",
    Desc => "desc",
});

ast_record!(query::TextAnalyzer, "semantic:query:TextAnalyzer", {
    stemming: bool => "stemming" [default Default::default()],
    min_token_len: u32 => "min_token_len" [default Default::default()],
});

ast_enum!(query::TextMatchMode, "semantic:query:TextMatchMode", {
    All => "all",
    Any => "any",
});

ast_enum!(query::TextQueryFormat, "semantic:query:TextQueryFormat", {
    Sql => "sql",
    Prql => "prql",
});

ast_enum!(query::UnaryOp, "semantic:query:UnaryOp", {
    Not => "not",
    Neg => "neg",
});

ast_record!(query::UpdateQuery, "semantic:query:UpdateQuery", {
    collection: Option<String> => "collection" [default None],
    predicate: Option<query::Expr> => "predicate" [default None],
    assignments: Vec<query::Assignment> => "assignments" [default Vec::new()],
    limit: Option<query::Expr> => "limit" [default None],
    returning: Vec<query::QueryField> => "returning" [default Vec::new()],
    field_format: query::FieldFormat => "field_format" [default query::FieldFormat::default()],
});
