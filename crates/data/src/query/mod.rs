#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Concat,
    And,
    Or,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    In,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum PatternMatchKind {
    Like,
    SimilarTo,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum AggregateOp {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FunctionArg<T> {
    Expr(T),
    Wildcard,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

use std::collections::BTreeMap;

use crate::value::{FieldPath, Object, Value};

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Operand {
    Field(FieldPath),
    Literal(Value),
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Expr {
    Operand(Operand),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    IfElse {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Coalesce(Vec<Expr>),
    Function {
        name: String,
        args: Vec<FunctionArg<Expr>>,
    },
    Aggregate {
        op: AggregateOp,
        distinct: bool,
        arg: Box<FunctionArg<Expr>>,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
    },
    Subquery(Box<SelectQuery>),
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
    },
    PatternMatch {
        kind: PatternMatchKind,
        expr: Box<Expr>,
        pattern: Box<Expr>,
        case_insensitive: bool,
        negated: bool,
    },
    RegexMatch {
        expr: Box<Expr>,
        pattern: Box<Expr>,
        case_insensitive: bool,
        negated: bool,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
    Exists {
        query: Box<SelectQuery>,
        negated: bool,
    },
    RelationExists {
        relation: Box<Expr>,
        source: Box<Expr>,
        target: Box<Expr>,
        transitive: bool,
        max_depth: Option<Box<Expr>>,
    },
}

impl From<usize> for Expr {
    fn from(value: usize) -> Self {
        Self::Operand(Operand::Literal(Value::U64(value as u64)))
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct QueryField {
    pub expr: Box<Expr>,
    pub alias: Option<String>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub expr: Expr,
    pub direction: SortDirection,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum JoinCondition {
    OnExpr(Expr),
    UsingFields { left: FieldPath, right: FieldPath },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct JoinSource {
    pub collection: Option<String>,
    pub class: Option<String>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct JoinQuery {
    pub source: JoinSource,
    pub alias: Option<String>,
    pub join_type: JoinType,
    pub condition: JoinCondition,
    pub predicate: Option<Expr>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct SelectQuery {
    pub collection: Option<String>,
    pub source_alias: Option<String>,
    pub joins: Vec<JoinQuery>,
    pub predicate: Option<Expr>,
    pub projection: Vec<QueryField>,
    pub distinct: bool,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub order_by: Vec<OrderBy>,
    pub offset: Expr,
    pub limit: Option<Expr>,
    pub field_format: FieldFormat,
}

impl SelectQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            source_alias: None,
            joins: Vec::new(),
            predicate: None,
            projection: Vec::new(),
            distinct: false,
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            offset: Expr::from(0usize),
            limit: None,
            field_format: FieldFormat::Plain,
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = Some(collection.into());
        self
    }

    pub fn with_predicate(mut self, predicate: Expr) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn with_source_alias(mut self, alias: impl Into<String>) -> Self {
        self.source_alias = Some(alias.into());
        self
    }

    pub fn with_joins(mut self, joins: Vec<JoinQuery>) -> Self {
        self.joins = joins;
        self
    }

    pub fn with_projection(mut self, projection: Vec<QueryField>) -> Self {
        self.projection = projection;
        self
    }

    pub fn with_distinct(mut self, distinct: bool) -> Self {
        self.distinct = distinct;
        self
    }

    pub fn with_group_by(mut self, group_by: Vec<Expr>) -> Self {
        self.group_by = group_by;
        self
    }

    pub fn with_having(mut self, having: Expr) -> Self {
        self.having = Some(having);
        self
    }

    pub fn with_order_by(mut self, order_by: Vec<OrderBy>) -> Self {
        self.order_by = order_by;
        self
    }

    pub fn with_limit(mut self, limit: impl Into<Expr>) -> Self {
        self.limit = Some(limit.into());
        self
    }

    pub fn with_offset(mut self, offset: impl Into<Expr>) -> Self {
        self.offset = offset.into();
        self
    }

    pub fn with_field_format(mut self, field_format: FieldFormat) -> Self {
        self.field_format = field_format;
        self
    }
}

impl Default for SelectQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct Assignment {
    pub path: FieldPath,
    pub value: Expr,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum InsertSource {
    Objects(Vec<Object>),
    Values(Vec<Vec<Expr>>),
    Select(SelectQuery),
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct InsertQuery {
    pub collection: Option<String>,
    pub columns: Vec<String>,
    pub source: InsertSource,
    pub returning: Vec<QueryField>,
    pub field_format: FieldFormat,
}

impl InsertQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            columns: Vec::new(),
            source: InsertSource::Objects(Vec::new()),
            returning: Vec::new(),
            field_format: FieldFormat::Plain,
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = Some(collection.into());
        self
    }

    pub fn with_columns(mut self, columns: Vec<String>) -> Self {
        self.columns = columns;
        self
    }

    pub fn with_source(mut self, source: InsertSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_returning(mut self, projection: Vec<QueryField>) -> Self {
        self.returning = projection;
        self
    }

    pub fn with_field_format(mut self, field_format: FieldFormat) -> Self {
        self.field_format = field_format;
        self
    }
}

impl Default for InsertQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct UpdateQuery {
    pub collection: Option<String>,
    pub predicate: Option<Expr>,
    pub assignments: Vec<Assignment>,
    pub limit: Option<Expr>,
    pub returning: Vec<QueryField>,
    pub field_format: FieldFormat,
}

impl UpdateQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            predicate: None,
            assignments: Vec::new(),
            limit: None,
            returning: Vec::new(),
            field_format: FieldFormat::Plain,
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = Some(collection.into());
        self
    }

    pub fn with_predicate(mut self, predicate: Expr) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn set(mut self, path: FieldPath, value: Expr) -> Self {
        self.assignments.push(Assignment { path, value });
        self
    }

    pub fn with_limit(mut self, limit: impl Into<Expr>) -> Self {
        self.limit = Some(limit.into());
        self
    }

    pub fn with_returning(mut self, projection: Vec<QueryField>) -> Self {
        self.returning = projection;
        self
    }

    pub fn with_field_format(mut self, field_format: FieldFormat) -> Self {
        self.field_format = field_format;
        self
    }
}

impl Default for UpdateQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct DeleteQuery {
    pub collection: Option<String>,
    pub predicate: Option<Expr>,
    pub limit: Option<Expr>,
    pub returning: Vec<QueryField>,
    pub field_format: FieldFormat,
}

impl DeleteQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            predicate: None,
            limit: None,
            returning: Vec::new(),
            field_format: FieldFormat::Plain,
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = Some(collection.into());
        self
    }

    pub fn with_predicate(mut self, predicate: Expr) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn with_limit(mut self, limit: impl Into<Expr>) -> Self {
        self.limit = Some(limit.into());
        self
    }

    pub fn with_returning(mut self, projection: Vec<QueryField>) -> Self {
        self.returning = projection;
        self
    }

    pub fn with_field_format(mut self, field_format: FieldFormat) -> Self {
        self.field_format = field_format;
        self
    }
}

impl Default for DeleteQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Query {
    Select(SelectQuery),
    Insert(InsertQuery),
    Update(UpdateQuery),
    Delete(DeleteQuery),
}

impl From<SelectQuery> for Query {
    fn from(value: SelectQuery) -> Self {
        Self::Select(value)
    }
}

impl From<UpdateQuery> for Query {
    fn from(value: UpdateQuery) -> Self {
        Self::Update(value)
    }
}

impl From<InsertQuery> for Query {
    fn from(value: InsertQuery) -> Self {
        Self::Insert(value)
    }
}

impl From<DeleteQuery> for Query {
    fn from(value: DeleteQuery) -> Self {
        Self::Delete(value)
    }
}

impl Query {
    pub fn collection(&self) -> Option<&str> {
        match self {
            Self::Select(query) => query.collection.as_deref(),
            Self::Insert(query) => query.collection.as_deref(),
            Self::Update(query) => query.collection.as_deref(),
            Self::Delete(query) => query.collection.as_deref(),
        }
    }
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TextQueryFormat {
    Sql,
    Prql,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FieldFormat {
    Qualified,
    Underscore,
    Plain,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum QueryInput {
    Ast(Query),
    Text {
        format: TextQueryFormat,
        query: String,
    },
}

impl QueryInput {
    pub fn sql(query: impl Into<String>) -> Self {
        Self::Text {
            format: TextQueryFormat::Sql,
            query: query.into(),
        }
    }

    pub fn prql(query: impl Into<String>) -> Self {
        Self::Text {
            format: TextQueryFormat::Prql,
            query: query.into(),
        }
    }
}

impl From<Query> for QueryInput {
    fn from(value: Query) -> Self {
        Self::Ast(value)
    }
}

impl From<SelectQuery> for QueryInput {
    fn from(value: SelectQuery) -> Self {
        Self::Ast(Query::Select(value))
    }
}

impl From<UpdateQuery> for QueryInput {
    fn from(value: UpdateQuery) -> Self {
        Self::Ast(Query::Update(value))
    }
}

impl From<DeleteQuery> for QueryInput {
    fn from(value: DeleteQuery) -> Self {
        Self::Ast(Query::Delete(value))
    }
}

impl From<InsertQuery> for QueryInput {
    fn from(value: InsertQuery) -> Self {
        Self::Ast(Query::Insert(value))
    }
}

impl From<String> for QueryInput {
    fn from(value: String) -> Self {
        Self::sql(value)
    }
}

impl From<&str> for QueryInput {
    fn from(value: &str) -> Self {
        Self::sql(value.to_string())
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct BatchStats {
    pub upserted: usize,
    pub deleted: usize,
    pub updated: usize,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BatchOperation {
    Upsert {
        collection: String,
        id: String,
        object: Object,
    },
    DeleteById {
        collection: String,
        id: String,
    },
    DeleteByIds {
        collection: String,
        ids: Vec<String>,
    },
    Update {
        collection: String,
        query: UpdateQuery,
    },
    Delete {
        collection: String,
        query: DeleteQuery,
    },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct Batch {
    pub operations: Vec<BatchOperation>,
}

impl Batch {
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    pub fn with_op(mut self, op: BatchOperation) -> Self {
        self.operations.push(op);
        self
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct BatchOutcome {
    pub dataset: BTreeMap<String, BTreeMap<String, Object>>,
    pub stats: BatchStats,
}
