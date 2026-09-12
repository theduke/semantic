#[cfg(feature = "sql")]
#[path = "query/sql/enabled.rs"]
pub mod sql;

#[cfg(not(feature = "sql"))]
#[path = "query/sql/disabled.rs"]
pub mod sql;

#[cfg(feature = "prql")]
#[path = "query/prql/enabled.rs"]
pub mod prql;

#[cfg(not(feature = "prql"))]
#[path = "query/prql/disabled.rs"]
pub mod prql;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextQueryFormat {
    Sql,
    Prql,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextQueryInput {
    Ast(Query),
    Text {
        format: TextQueryFormat,
        query: String,
        params: BTreeMap<String, Value>,
    },
}

impl TextQueryInput {
    pub fn sql_with_params(query: impl Into<String>, params: BTreeMap<String, Value>) -> Self {
        Self::Text {
            format: TextQueryFormat::Sql,
            query: query.into(),
            params,
        }
    }

    pub fn sql(query: impl Into<String>) -> Self {
        Self::Text {
            format: TextQueryFormat::Sql,
            query: query.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn prql(query: impl Into<String>) -> Self {
        Self::Text {
            format: TextQueryFormat::Prql,
            query: query.into(),
            params: BTreeMap::new(),
        }
    }
}

impl From<Query> for TextQueryInput {
    fn from(value: Query) -> Self {
        Self::Ast(value)
    }
}

impl From<SelectQuery> for TextQueryInput {
    fn from(value: SelectQuery) -> Self {
        Self::Ast(Query::Select(value))
    }
}

impl From<UpdateQuery> for TextQueryInput {
    fn from(value: UpdateQuery) -> Self {
        Self::Ast(Query::Update(value))
    }
}

impl From<DeleteQuery> for TextQueryInput {
    fn from(value: DeleteQuery) -> Self {
        Self::Ast(Query::Delete(value))
    }
}

impl From<InsertQuery> for TextQueryInput {
    fn from(value: InsertQuery) -> Self {
        Self::Ast(Query::Insert(value))
    }
}

impl From<DdlQuery> for TextQueryInput {
    fn from(value: DdlQuery) -> Self {
        Self::Ast(Query::Ddl(value))
    }
}

impl From<String> for TextQueryInput {
    fn from(value: String) -> Self {
        Self::sql(value)
    }
}

impl From<&str> for TextQueryInput {
    fn from(value: &str) -> Self {
        Self::sql(value.to_string())
    }
}

impl From<public_query::TextQueryFormat> for TextQueryFormat {
    fn from(value: public_query::TextQueryFormat) -> Self {
        match value {
            public_query::TextQueryFormat::Sql => Self::Sql,
            public_query::TextQueryFormat::Prql => Self::Prql,
        }
    }
}

impl From<TextQueryFormat> for public_query::TextQueryFormat {
    fn from(value: TextQueryFormat) -> Self {
        match value {
            TextQueryFormat::Sql => Self::Sql,
            TextQueryFormat::Prql => Self::Prql,
        }
    }
}

impl From<public_query::QueryInput> for TextQueryInput {
    fn from(value: public_query::QueryInput) -> Self {
        match value {
            public_query::QueryInput::Ast(query) => Self::Ast(query.into()),
            public_query::QueryInput::Text {
                format,
                query,
                params,
            } => Self::Text {
                format: format.into(),
                query,
                params,
            },
        }
    }
}

pub type QueryInput = TextQueryInput;

pub use prql::*;
pub use sql::*;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
};

use regex::RegexBuilder;
use semantic_data::query as public_query;
use semantic_data::query::{
    AggregateOp, BinaryOp, FieldFormat, JoinType, PatternMatchKind, SortDirection, UnaryOp,
};
use semantic_data::value::{FieldPath, Object, PathSegment, Value, ValueRef};

use crate::DdlBatch;
use crate::catalog::{LocalAttrId, LocalCollectionId, LocalFieldId};

pub trait ValueAccess {
    fn as_value_ref(&self) -> ValueRef<'_>;

    fn to_owned_value(&self) -> Value {
        self.as_value_ref().into_owned()
    }
}

impl ValueAccess for Value {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::Ref(self)
    }
}

impl ValueAccess for ValueRef<'_> {
    fn as_value_ref(&self) -> ValueRef<'_> {
        self.clone()
    }
}

pub trait ObjectAccess: Send + Sync {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>>;

    fn value_at_attr_ref<'a>(&'a self, _attr: LocalAttrId) -> Option<ValueRef<'a>> {
        None
    }

    fn value_at_field_ref<'a>(&'a self, _field: LocalFieldId) -> Option<ValueRef<'a>> {
        None
    }

    fn collection_id(&self) -> Option<LocalCollectionId> {
        None
    }

    fn to_object(&self) -> Object;
}

impl ObjectAccess for Object {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>> {
        map_value_at_path_ref(self, path)
    }

    fn to_object(&self) -> Object {
        self.clone()
    }
}

impl ObjectAccess for BTreeMap<String, Value> {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>> {
        map_value_at_path_ref(self, path)
    }

    fn to_object(&self) -> Object {
        self.clone().into()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Operand {
    Field(FieldPath),
    Literal(Value),
}

pub type FunctionArg = semantic_data::query::FunctionArg<Expr>;

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
        args: Vec<FunctionArg>,
    },
    Aggregate {
        op: AggregateOp,
        distinct: bool,
        arg: Box<FunctionArg>,
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
    pub wildcard: Option<FieldPath>,
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

impl JoinSource {
    pub fn relation_name(&self) -> String {
        match (&self.collection, &self.class) {
            (None, None) => "_".to_string(),
            (None, Some(class)) => class.clone(),
            (Some(collection), None) => format!("{collection}._"),
            (Some(collection), Some(class)) => format!("{collection}.{class}"),
        }
    }

    pub fn default_binding(&self) -> String {
        self.alias_seed().to_string()
    }

    fn alias_seed(&self) -> &str {
        if let Some(class) = self.class.as_deref() {
            class
        } else if let Some(collection) = self.collection.as_deref() {
            collection
        } else {
            "_"
        }
    }
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

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct DdlQuery {
    pub batch: DdlBatch,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Query {
    Select(SelectQuery),
    Insert(InsertQuery),
    Update(UpdateQuery),
    Delete(DeleteQuery),
    Ddl(DdlQuery),
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

impl From<DdlQuery> for Query {
    fn from(value: DdlQuery) -> Self {
        Self::Ddl(value)
    }
}

impl From<public_query::Operand> for Operand {
    fn from(value: public_query::Operand) -> Self {
        match value {
            public_query::Operand::Field(path) => Self::Field(path),
            public_query::Operand::Literal(value) => Self::Literal(value),
        }
    }
}

impl From<public_query::Expr> for Expr {
    fn from(value: public_query::Expr) -> Self {
        match value {
            public_query::Expr::Operand(operand) => Self::Operand(operand.into()),
            public_query::Expr::Unary { op, expr } => Self::Unary {
                op,
                expr: Box::new((*expr).into()),
            },
            public_query::Expr::Binary { op, left, right } => Self::Binary {
                op,
                left: Box::new((*left).into()),
                right: Box::new((*right).into()),
            },
            public_query::Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => Self::IfElse {
                cond: Box::new((*cond).into()),
                then_expr: Box::new((*then_expr).into()),
                else_expr: Box::new((*else_expr).into()),
            },
            public_query::Expr::Coalesce(items) => {
                Self::Coalesce(items.into_iter().map(Into::into).collect())
            }
            public_query::Expr::Function { name, args } => Self::Function {
                name,
                args: args
                    .into_iter()
                    .map(|arg| match arg {
                        public_query::FunctionArg::Expr(expr) => FunctionArg::Expr(expr.into()),
                        public_query::FunctionArg::Wildcard => FunctionArg::Wildcard,
                    })
                    .collect(),
            },
            public_query::Expr::Aggregate { op, distinct, arg } => Self::Aggregate {
                op,
                distinct,
                arg: Box::new(match *arg {
                    public_query::FunctionArg::Expr(expr) => FunctionArg::Expr(expr.into()),
                    public_query::FunctionArg::Wildcard => FunctionArg::Wildcard,
                }),
            },
            public_query::Expr::InList {
                expr,
                list,
                negated,
            } => Self::InList {
                expr: Box::new((*expr).into()),
                list: list.into_iter().map(Into::into).collect(),
                negated,
            },
            public_query::Expr::Subquery(query) => Self::Subquery(Box::new((*query).into())),
            public_query::Expr::Between {
                expr,
                low,
                high,
                negated,
            } => Self::Between {
                expr: Box::new((*expr).into()),
                low: Box::new((*low).into()),
                high: Box::new((*high).into()),
                negated,
            },
            public_query::Expr::PatternMatch {
                kind,
                expr,
                pattern,
                case_insensitive,
                negated,
            } => Self::PatternMatch {
                kind,
                expr: Box::new((*expr).into()),
                pattern: Box::new((*pattern).into()),
                case_insensitive,
                negated,
            },
            public_query::Expr::RegexMatch {
                expr,
                pattern,
                case_insensitive,
                negated,
            } => Self::RegexMatch {
                expr: Box::new((*expr).into()),
                pattern: Box::new((*pattern).into()),
                case_insensitive,
                negated,
            },
            public_query::Expr::IsNull { expr, negated } => Self::IsNull {
                expr: Box::new((*expr).into()),
                negated,
            },
            public_query::Expr::Exists { query, negated } => Self::Exists {
                query: Box::new((*query).into()),
                negated,
            },
            public_query::Expr::RelationExists {
                relation,
                source,
                target,
                transitive,
                max_depth,
            } => Self::RelationExists {
                relation: Box::new((*relation).into()),
                source: Box::new((*source).into()),
                target: Box::new((*target).into()),
                transitive,
                max_depth: max_depth.map(|value| Box::new((*value).into())),
            },
        }
    }
}

impl From<public_query::QueryField> for QueryField {
    fn from(value: public_query::QueryField) -> Self {
        Self {
            expr: Box::new((*value.expr).into()),
            alias: value.alias,
            wildcard: None,
        }
    }
}

impl From<public_query::OrderBy> for OrderBy {
    fn from(value: public_query::OrderBy) -> Self {
        Self {
            expr: value.expr.into(),
            direction: value.direction,
        }
    }
}

impl From<public_query::JoinCondition> for JoinCondition {
    fn from(value: public_query::JoinCondition) -> Self {
        match value {
            public_query::JoinCondition::OnExpr(expr) => Self::OnExpr(expr.into()),
            public_query::JoinCondition::UsingFields { left, right } => {
                Self::UsingFields { left, right }
            }
        }
    }
}

impl From<public_query::JoinQuery> for JoinQuery {
    fn from(value: public_query::JoinQuery) -> Self {
        Self {
            source: JoinSource {
                collection: value.source.collection,
                class: value.source.class,
            },
            alias: value.alias,
            join_type: value.join_type,
            condition: value.condition.into(),
            predicate: value.predicate.map(Into::into),
        }
    }
}

impl From<public_query::SelectQuery> for SelectQuery {
    fn from(value: public_query::SelectQuery) -> Self {
        Self {
            collection: value.collection,
            source_alias: value.source_alias,
            joins: value.joins.into_iter().map(Into::into).collect(),
            predicate: value.predicate.map(Into::into),
            projection: value.projection.into_iter().map(Into::into).collect(),
            distinct: value.distinct,
            group_by: value.group_by.into_iter().map(Into::into).collect(),
            having: value.having.map(Into::into),
            order_by: value.order_by.into_iter().map(Into::into).collect(),
            offset: value.offset.into(),
            limit: value.limit.map(Into::into),
            field_format: value.field_format.into(),
        }
    }
}

impl From<public_query::Assignment> for Assignment {
    fn from(value: public_query::Assignment) -> Self {
        Self {
            path: value.path,
            value: value.value.into(),
        }
    }
}

impl From<public_query::InsertSource> for InsertSource {
    fn from(value: public_query::InsertSource) -> Self {
        match value {
            public_query::InsertSource::Objects(objects) => Self::Objects(objects),
            public_query::InsertSource::Values(rows) => Self::Values(
                rows.into_iter()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
            ),
            public_query::InsertSource::Select(query) => Self::Select(query.into()),
        }
    }
}

impl From<public_query::InsertQuery> for InsertQuery {
    fn from(value: public_query::InsertQuery) -> Self {
        Self {
            collection: value.collection,
            columns: value.columns,
            source: value.source.into(),
            returning: value.returning.into_iter().map(Into::into).collect(),
            field_format: value.field_format.into(),
        }
    }
}

impl From<public_query::UpdateQuery> for UpdateQuery {
    fn from(value: public_query::UpdateQuery) -> Self {
        Self {
            collection: value.collection,
            predicate: value.predicate.map(Into::into),
            assignments: value.assignments.into_iter().map(Into::into).collect(),
            limit: value.limit.map(Into::into),
            returning: value.returning.into_iter().map(Into::into).collect(),
            field_format: value.field_format.into(),
        }
    }
}

impl From<public_query::DeleteQuery> for DeleteQuery {
    fn from(value: public_query::DeleteQuery) -> Self {
        Self {
            collection: value.collection,
            predicate: value.predicate.map(Into::into),
            limit: value.limit.map(Into::into),
            returning: value.returning.into_iter().map(Into::into).collect(),
            field_format: value.field_format.into(),
        }
    }
}

impl From<public_query::Query> for Query {
    fn from(value: public_query::Query) -> Self {
        match value {
            public_query::Query::Select(query) => Self::Select(query.into()),
            public_query::Query::Insert(query) => Self::Insert(query.into()),
            public_query::Query::Update(query) => Self::Update(query.into()),
            public_query::Query::Delete(query) => Self::Delete(query.into()),
        }
    }
}

impl From<public_query::BatchOperation> for BatchOperation {
    fn from(value: public_query::BatchOperation) -> Self {
        match value {
            public_query::BatchOperation::Upsert {
                collection,
                id,
                object,
            } => Self::Upsert {
                collection,
                id,
                object,
            },
            public_query::BatchOperation::Create {
                collection,
                id,
                object,
            } => Self::Create {
                collection,
                id,
                object,
            },
            public_query::BatchOperation::DeleteById { collection, id } => {
                Self::DeleteById { collection, id }
            }
            public_query::BatchOperation::DeleteByIds { collection, ids } => {
                Self::DeleteByIds { collection, ids }
            }
            public_query::BatchOperation::Update { collection, query } => Self::Update {
                collection,
                query: query.into(),
            },
            public_query::BatchOperation::Delete { collection, query } => Self::Delete {
                collection,
                query: query.into(),
            },
        }
    }
}

impl From<public_query::Batch> for Batch {
    fn from(value: public_query::Batch) -> Self {
        Self {
            operations: value.operations.into_iter().map(Into::into).collect(),
        }
    }
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

    pub fn collection_or_default(&self) -> &str {
        self.collection
            .as_deref()
            .unwrap_or(crate::DEFAULT_COLLECTION)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MachinePhase {
    Collecting,
    Finalized,
}

#[derive(Debug, Clone)]
pub struct QueryStateMachine {
    query: SelectQuery,
    phase: MachinePhase,
    collected: Vec<Object>,
    out: Vec<Object>,
    consumed_offset: usize,
    produced: usize,
}

impl QueryStateMachine {
    pub fn new(query: SelectQuery) -> Self {
        Self {
            query,
            phase: MachinePhase::Collecting,
            collected: Vec::new(),
            out: Vec::new(),
            consumed_offset: 0,
            produced: 0,
        }
    }

    pub fn is_done(&self) -> bool {
        self.phase == MachinePhase::Finalized
            && self.out.is_empty()
            && self
                .query
                .limit
                .as_ref()
                .and_then(evaluate_usize_expr)
                .map(|limit| self.produced >= limit)
                .unwrap_or(false)
    }

    pub fn push_row(&mut self, row: Object) {
        if self.phase != MachinePhase::Collecting {
            return;
        }

        if !row_matches(&row, &self.query.predicate) {
            return;
        }

        if self.query.order_by.is_empty() {
            self.push_result_row(row);
        } else {
            self.collected.push(row);
        }
    }

    pub fn finish(&mut self) {
        if self.phase != MachinePhase::Collecting {
            return;
        }

        if !self.query.order_by.is_empty() {
            self.collected
                .sort_by(|a, b| compare_rows(a, b, &self.query.order_by));
            let rows = std::mem::take(&mut self.collected);
            for row in rows {
                self.push_result_row(row);
            }
        }

        self.phase = MachinePhase::Finalized;
    }

    pub fn drain_ready(&mut self) -> Vec<Object> {
        std::mem::take(&mut self.out)
    }

    fn push_result_row(&mut self, row: Object) {
        let offset = evaluate_usize_expr(&self.query.offset).unwrap_or(0);
        if self.consumed_offset < offset {
            self.consumed_offset += 1;
            return;
        }

        if let Some(limit) = self.query.limit.as_ref().and_then(evaluate_usize_expr) {
            if self.produced >= limit {
                return;
            }
        }

        let out = if self.query.projection.is_empty() {
            row
        } else {
            project_object(&row, &self.query.projection)
        };

        self.out.push(out);
        self.produced += 1;
    }
}

pub fn execute_query(query: &SelectQuery, rows: impl IntoIterator<Item = Object>) -> Vec<Object> {
    let mut machine = QueryStateMachine::new(query.clone());
    for row in rows {
        machine.push_row(row);
    }
    machine.finish();
    machine.drain_ready()
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

    pub fn collection_or_default(&self) -> &str {
        self.collection
            .as_deref()
            .unwrap_or(crate::DEFAULT_COLLECTION)
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

    pub fn collection_or_default(&self) -> &str {
        self.collection
            .as_deref()
            .unwrap_or(crate::DEFAULT_COLLECTION)
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

    pub fn collection_or_default(&self) -> &str {
        self.collection
            .as_deref()
            .unwrap_or(crate::DEFAULT_COLLECTION)
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

impl Query {
    pub fn collection(&self) -> Option<&str> {
        match self {
            Self::Select(query) => query.collection.as_deref(),
            Self::Insert(query) => query.collection.as_deref(),
            Self::Update(query) => query.collection.as_deref(),
            Self::Delete(query) => query.collection.as_deref(),
            Self::Ddl(_) => None,
        }
    }

    pub fn collection_or_default(&self) -> &str {
        self.collection().unwrap_or(crate::DEFAULT_COLLECTION)
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct MutationStats {
    pub matched: usize,
    pub affected: usize,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct InsertResult {
    pub inserted: usize,
    pub returning: Vec<Object>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct UpdateResult {
    pub stats: MutationStats,
    pub returning: Vec<Object>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct DeleteResult {
    pub deleted: usize,
    pub returning: Vec<Object>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum QueryResult {
    Select(Vec<Object>),
    Insert(InsertResult),
    Update(UpdateResult),
    Delete(DeleteResult),
    Ddl(()),
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct BatchStats {
    pub upserted: usize,
    pub deleted: usize,
    pub updated: usize,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub id: String,
    pub collection: String,
    pub object: Object,
}

pub type Dataset = BTreeMap<String, BTreeMap<String, Object>>;

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BatchOperation {
    Create {
        collection: String,
        id: String,
        object: Object,
    },
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
    pub dataset: Dataset,
    pub stats: BatchStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub message: String,
    pub entity_exists: Option<(String, String)>,
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            entity_exists: None,
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for CoreError {}

pub type CoreResult<T> = std::result::Result<T, CoreError>;

pub fn execute_batch(input: &Dataset, batch: &Batch) -> CoreResult<BatchOutcome> {
    execute_batch_with_prepare(input, batch, |_, _, _| Ok(()))
}

pub fn execute_batch_with_prepare<F>(
    input: &Dataset,
    batch: &Batch,
    mut prepare: F,
) -> CoreResult<BatchOutcome>
where
    F: FnMut(&str, &str, &mut Object) -> CoreResult<()>,
{
    // Validate every mutation limit before cloning or inspecting the dataset. In
    // particular, an invalid programmatically-constructed DELETE must not be
    // interpreted as an unbounded mutation.
    for operation in &batch.operations {
        match operation {
            BatchOperation::Update { query, .. } => {
                evaluate_mutation_limit(query.limit.as_ref())?;
            }
            BatchOperation::Delete { query, .. } => {
                evaluate_mutation_limit(query.limit.as_ref())?;
            }
            _ => {}
        }
    }

    let mut dataset = input.clone();
    let mut stats = BatchStats {
        upserted: 0,
        deleted: 0,
        updated: 0,
    };

    for op in &batch.operations {
        match op {
            BatchOperation::Upsert {
                collection,
                id,
                object,
            } => {
                let coll = dataset.entry(collection.clone()).or_default();
                let mut object = object.clone();
                prepare(collection, id, &mut object)?;
                coll.insert(id.clone(), object);
                stats.upserted += 1;
            }
            BatchOperation::Create {
                collection,
                id,
                object,
            } => {
                let coll = dataset.entry(collection.clone()).or_default();
                if coll.contains_key(id) {
                    return Err(CoreError {
                        message: format!(
                            "entity '{id}' already exists in collection '{collection}'"
                        ),
                        entity_exists: Some((collection.clone(), id.clone())),
                    });
                }
                let mut object = object.clone();
                prepare(collection, id, &mut object)?;
                coll.insert(id.clone(), object);
                stats.upserted += 1;
            }
            BatchOperation::DeleteById { collection, id } => {
                if let Some(coll) = dataset.get_mut(collection) {
                    if coll.remove(id).is_some() {
                        stats.deleted += 1;
                    }
                }
            }
            BatchOperation::DeleteByIds { collection, ids } => {
                if let Some(coll) = dataset.get_mut(collection) {
                    for id in ids {
                        if coll.remove(id).is_some() {
                            stats.deleted += 1;
                        }
                    }
                }
            }
            BatchOperation::Update { collection, query } => {
                if let Some(coll) = dataset.get_mut(collection) {
                    let mut entities = coll
                        .iter()
                        .map(|(id, object)| Entity {
                            id: id.clone(),
                            collection: collection.clone(),
                            object: object.clone(),
                        })
                        .collect::<Vec<_>>();

                    let result = apply_update_with_returning_and_prepare(
                        query,
                        &mut entities,
                        |id, object| prepare(collection, id, object),
                    )?;
                    stats.updated += result.stats.affected;

                    coll.clear();
                    for entity in entities {
                        coll.insert(entity.id, entity.object);
                    }
                }
            }
            BatchOperation::Delete { collection, query } => {
                if let Some(coll) = dataset.get_mut(collection) {
                    let entities = coll
                        .iter()
                        .map(|(id, object)| Entity {
                            id: id.clone(),
                            collection: collection.clone(),
                            object: object.clone(),
                        })
                        .collect::<Vec<_>>();

                    let (remaining, deleted) = apply_delete(query, entities);
                    stats.deleted += deleted;

                    coll.clear();
                    for entity in remaining {
                        coll.insert(entity.id, entity.object);
                    }
                }
            }
        }
    }

    Ok(BatchOutcome { dataset, stats })
}

pub fn apply_update(query: &UpdateQuery, entities: &mut [Entity]) -> CoreResult<MutationStats> {
    apply_update_with_returning(query, entities).map(|result| result.stats)
}

pub fn apply_update_with_returning(
    query: &UpdateQuery,
    entities: &mut [Entity],
) -> CoreResult<UpdateResult> {
    apply_update_with_returning_and_prepare(query, entities, |_, _| Ok(()))
}

pub fn apply_update_with_returning_and_prepare<F>(
    query: &UpdateQuery,
    entities: &mut [Entity],
    mut prepare: F,
) -> CoreResult<UpdateResult>
where
    F: FnMut(&str, &mut Object) -> CoreResult<()>,
{
    let limit = evaluate_mutation_limit(query.limit.as_ref())?;
    let mut matched = 0usize;
    let mut affected = 0usize;
    let mut returning = Vec::new();
    let mut staged = Vec::new();

    for (index, entity) in entities.iter().enumerate() {
        if !row_matches(&entity.object, &query.predicate) {
            continue;
        }

        if let Some(limit) = limit {
            if matched >= limit {
                break;
            }
        }

        matched += 1;
        let mut updated = entity.object.clone();

        for assignment in &query.assignments {
            // SQL assignments are simultaneous: every right-hand side observes
            // the row as it was before this UPDATE, not earlier assignments.
            let value = evaluate_expr(&entity.object, &assignment.value)
                .ok_or_else(|| CoreError::new("failed to evaluate assignment expression"))?;
            set_value_at_path(&mut updated, &assignment.path, value)?;
        }

        prepare(&entity.id, &mut updated)?;

        if !query.returning.is_empty() {
            returning.push(project_object(&updated, &query.returning));
        }

        if updated != entity.object {
            affected += 1;
        }
        staged.push((index, updated));
    }

    // Commit only after every matching row has evaluated and validated. This
    // preserves statement atomicity for callers of the public core API too.
    for (index, updated) in staged {
        entities[index].object = updated;
    }

    Ok(UpdateResult {
        stats: MutationStats { matched, affected },
        returning,
    })
}

pub fn apply_delete(query: &DeleteQuery, entities: Vec<Entity>) -> (Vec<Entity>, usize) {
    let (remaining, result) = apply_delete_with_remaining(query, entities);
    (remaining, result.deleted)
}

struct DeletePlanResult {
    remaining: Vec<Entity>,
    deleted: usize,
    returning: Vec<Object>,
}

pub fn apply_delete_with_returning(query: &DeleteQuery, entities: Vec<Entity>) -> DeleteResult {
    apply_delete_with_remaining(query, entities).1
}

/// Applies a delete in one traversal, returning both surviving entities and
/// the mutation result (including any RETURNING projection).
pub fn apply_delete_with_remaining(
    query: &DeleteQuery,
    entities: Vec<Entity>,
) -> (Vec<Entity>, DeleteResult) {
    let DeletePlanResult {
        remaining,
        deleted,
        returning,
    } = apply_delete_plan(query, entities);
    (remaining, DeleteResult { deleted, returning })
}

fn apply_delete_plan(query: &DeleteQuery, entities: Vec<Entity>) -> DeletePlanResult {
    let mut deleted = 0usize;
    let mut remaining = Vec::with_capacity(entities.len());
    let mut returning = Vec::new();
    // The legacy standalone delete helpers cannot return an error. Treat an
    // invalid limit as zero so they fail closed instead of deleting all rows.
    let limit = evaluate_mutation_limit(query.limit.as_ref()).unwrap_or(Some(0));

    for entity in entities {
        if row_matches(&entity.object, &query.predicate)
            && limit.map(|value| deleted < value).unwrap_or(true)
        {
            deleted += 1;
            if !query.returning.is_empty() {
                returning.push(project_object(&entity.object, &query.returning));
            }
        } else {
            remaining.push(entity);
        }
    }

    DeletePlanResult {
        remaining,
        deleted,
        returning,
    }
}

pub fn evaluate_expr<T: ObjectAccess + ?Sized>(value: &T, expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Operand(operand) => resolve_operand(value, operand).map(|v| v.into_owned()),
        Expr::Unary { op, expr } => {
            let value = evaluate_expr(value, expr)?;
            match op {
                UnaryOp::Not => Some(Value::Bool(!value_truthy(&value))),
                UnaryOp::Neg => negate_value(value),
            }
        }
        Expr::Binary { op, left, right } => {
            let left = evaluate_expr(value, left)?;
            let right = evaluate_expr(value, right)?;
            eval_binary(*op, left, right)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            let c = evaluate_expr(value, cond)?;
            if value_truthy(&c) {
                evaluate_expr(value, then_expr)
            } else {
                evaluate_expr(value, else_expr)
            }
        }
        Expr::Coalesce(items) => {
            for item in items {
                let Some(v) = evaluate_expr(value, item) else {
                    continue;
                };
                if !v.is_nullish() {
                    return Some(v);
                }
            }
            Some(Value::Null)
        }
        Expr::Function { name, args } => evaluate_function(value, name, args),
        Expr::Aggregate { .. } => None,
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let Some(target) = evaluate_expr(value, expr) else {
                return Some(Value::Bool(false));
            };
            let mut found = false;
            for item in list {
                let Some(candidate) = evaluate_expr(value, item) else {
                    continue;
                };
                if candidate == target {
                    found = true;
                    break;
                }
            }
            Some(Value::Bool(if *negated { !found } else { found }))
        }
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::RelationExists { .. } => None,
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let Some(v) = evaluate_expr(value, expr) else {
                return Some(Value::Bool(false));
            };
            let Some(lo) = evaluate_expr(value, low) else {
                return Some(Value::Bool(false));
            };
            let Some(hi) = evaluate_expr(value, high) else {
                return Some(Value::Bool(false));
            };
            let in_range = v >= lo && v <= hi;
            Some(Value::Bool(if *negated { !in_range } else { in_range }))
        }
        Expr::PatternMatch {
            kind,
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let matched = evaluate_expr(value, expr)
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .zip(evaluate_expr(value, pattern).and_then(|v| v.as_str().map(ToOwned::to_owned)))
                .is_some_and(|(input, pattern)| match kind {
                    PatternMatchKind::Like => like_match(&input, &pattern, *case_insensitive),
                    PatternMatchKind::SimilarTo => {
                        similar_to_match(&input, &pattern, *case_insensitive)
                    }
                });
            Some(Value::Bool(if *negated { !matched } else { matched }))
        }
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let matched = evaluate_expr(value, expr)
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .zip(evaluate_expr(value, pattern).and_then(|v| v.as_str().map(ToOwned::to_owned)))
                .is_some_and(|(input, pattern)| {
                    regex_match(&input, &pattern, *case_insensitive).unwrap_or(false)
                });
            Some(Value::Bool(if *negated { !matched } else { matched }))
        }
        Expr::IsNull { expr, negated } => {
            let is_null = evaluate_expr(value, expr).is_none_or(|v| v.is_nullish());
            Some(Value::Bool(if *negated { !is_null } else { is_null }))
        }
    }
}

pub fn project_object<T: ObjectAccess + ?Sized>(value: &T, projection: &[QueryField]) -> Object {
    let mut out = Object::new();
    for project in projection {
        if let Some(path) = &project.wildcard {
            if path.segments().is_empty() {
                out.extend(value.to_object());
                continue;
            }
            let Some(Value::Object(object)) = value.value_at_path_ref(path).map(|v| v.into_owned())
            else {
                continue;
            };
            out.extend(object);
            continue;
        }
        let Some(v) = evaluate_expr(value, &project.expr) else {
            continue;
        };

        let key = project
            .alias
            .clone()
            .unwrap_or_else(|| infer_project_key(&project.expr));
        out.insert(key, v);
    }

    out
}

pub fn row_matches<T: ObjectAccess + ?Sized>(row: &T, predicate: &Option<Expr>) -> bool {
    match predicate {
        Some(predicate) => evaluate_filter_expr(row, predicate),
        None => true,
    }
}

pub fn evaluate_filter_expr<T: ObjectAccess + ?Sized>(row: &T, predicate: &Expr) -> bool {
    evaluate_expr(row, predicate)
        .as_ref()
        .is_some_and(value_truthy)
}

pub fn set_value_at_path(object: &mut Object, path: &FieldPath, value: Value) -> CoreResult<()> {
    if path.segments().is_empty() {
        return Err(CoreError::new("path cannot be empty"));
    }

    if path.segments().len() == 1 {
        match &path.segments()[0] {
            PathSegment::Field(field) => {
                object.insert(field.clone(), value);
                return Ok(());
            }
            PathSegment::Index(_) => {
                return Err(CoreError::new(
                    "top-level path segment for object updates must be a field",
                ));
            }
        }
    }

    let mut current: &mut Value = match &path.segments()[0] {
        PathSegment::Field(field) => object
            .entry(field.clone())
            .or_insert_with(|| Value::Object(Object::new())),
        PathSegment::Index(_) => {
            return Err(CoreError::new(
                "top-level path segment for object updates must be a field",
            ));
        }
    };

    for segment in path
        .segments()
        .iter()
        .skip(1)
        .take(path.segments().len().saturating_sub(2))
    {
        current = match segment {
            PathSegment::Field(field) => {
                if !matches!(current, Value::Object(_)) {
                    *current = Value::Object(Object::new());
                }

                let Value::Object(obj) = current else {
                    unreachable!()
                };
                obj.entry(field.clone())
                    .or_insert_with(|| Value::Object(Object::new()))
            }
            PathSegment::Index(index) => {
                if !matches!(current, Value::List(_)) {
                    *current = Value::List(Vec::new());
                }

                let Value::List(items) = current else {
                    unreachable!()
                };
                if *index >= items.len() {
                    items.resize(index + 1, Value::Null);
                }
                &mut items[*index]
            }
        };
    }

    let last = path
        .segments()
        .last()
        .ok_or_else(|| CoreError::new("path cannot be empty"))?;

    match last {
        PathSegment::Field(field) => {
            if !matches!(current, Value::Object(_)) {
                *current = Value::Object(Object::new());
            }
            let Value::Object(obj) = current else {
                unreachable!()
            };
            obj.insert(field.clone(), value);
        }
        PathSegment::Index(index) => {
            if !matches!(current, Value::List(_)) {
                *current = Value::List(Vec::new());
            }
            let Value::List(items) = current else {
                unreachable!()
            };
            if *index >= items.len() {
                items.resize(index + 1, Value::Null);
            }
            items[*index] = value;
        }
    }

    Ok(())
}

fn compare_rows(a: &Object, b: &Object, order_by: &[OrderBy]) -> Ordering {
    compare_objects(a, b, order_by)
}

pub(crate) fn compare_objects_for_plan(
    a: &dyn ObjectAccess,
    b: &dyn ObjectAccess,
    order_by: &[OrderBy],
) -> Ordering {
    compare_objects(a, b, order_by)
}

fn compare_objects<A: ObjectAccess + ?Sized, B: ObjectAccess + ?Sized>(
    a: &A,
    b: &B,
    order_by: &[OrderBy],
) -> Ordering {
    for order in order_by {
        let av = evaluate_expr(a, &order.expr);
        let bv = evaluate_expr(b, &order.expr);

        let ord = match (av.as_ref(), bv.as_ref()) {
            (Some(av), Some(bv)) => av.cmp(bv),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };

        if ord != Ordering::Equal {
            return match order.direction {
                SortDirection::Asc => ord,
                SortDirection::Desc => ord.reverse(),
            };
        }
    }

    Ordering::Equal
}

pub fn evaluate_usize_expr(expr: &Expr) -> Option<usize> {
    let value = evaluate_expr(&Object::new(), expr)?;
    match value {
        Value::I8(v) => usize::try_from(v).ok(),
        Value::I16(v) => usize::try_from(v).ok(),
        Value::I32(v) => usize::try_from(v).ok(),
        Value::I64(v) => usize::try_from(v).ok(),
        Value::I128(v) => usize::try_from(v).ok(),
        Value::U8(v) => Some(v as usize),
        Value::U16(v) => Some(v as usize),
        Value::U32(v) => usize::try_from(v).ok(),
        Value::U64(v) => usize::try_from(v).ok(),
        Value::U128(v) => usize::try_from(v).ok(),
        Value::F32(v) => {
            let value = v.into_inner();
            if value.is_finite()
                && value >= 0.0
                && value.fract() == 0.0
                && value <= usize::MAX as f32
            {
                Some(value as usize)
            } else {
                None
            }
        }
        Value::F64(v) => {
            let value = v.into_inner();
            if value.is_finite()
                && value >= 0.0
                && value.fract() == 0.0
                && value <= usize::MAX as f64
            {
                Some(value as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Evaluates a mutation limit, rejecting expressions that are not constant,
/// non-negative integers.
pub fn evaluate_mutation_limit(
    limit: Option<&Expr>,
) -> std::result::Result<Option<usize>, CoreError> {
    match limit {
        None => Ok(None),
        Some(expr) => evaluate_usize_expr(expr).map(Some).ok_or_else(|| {
            CoreError::new("mutation LIMIT must evaluate to a non-negative integer")
        }),
    }
}

fn resolve_operand<'a, T: ObjectAccess + ?Sized>(
    value: &'a T,
    operand: &'a Operand,
) -> Option<ValueRef<'a>> {
    match operand {
        Operand::Field(path) => value.value_at_path_ref(path),
        Operand::Literal(value) => Some(ValueRef::Ref(value)),
    }
}

fn infer_project_key(expr: &Expr) -> String {
    if let Expr::Operand(Operand::Field(path)) = expr {
        for segment in path.segments().iter().rev() {
            if let PathSegment::Field(name) = segment {
                return name.clone();
            }
        }
    }
    "value".to_string()
}

fn negate_value(value: Value) -> Option<Value> {
    match value {
        Value::I8(v) => Some(Value::I8(-v)),
        Value::I16(v) => Some(Value::I16(-v)),
        Value::I32(v) => Some(Value::I32(-v)),
        Value::I64(v) => Some(Value::I64(-v)),
        Value::I128(v) => Some(Value::I128(-v)),
        Value::F32(v) => Some(Value::F32((-v.into_inner()).into())),
        Value::F64(v) => Some(Value::F64((-v.into_inner()).into())),
        _ => None,
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    value.as_f64()
}

fn eval_binary(op: BinaryOp, left: Value, right: Value) -> Option<Value> {
    match op {
        BinaryOp::Add => Some(Value::F64((as_f64(&left)? + as_f64(&right)?).into())),
        BinaryOp::Sub => Some(Value::F64((as_f64(&left)? - as_f64(&right)?).into())),
        BinaryOp::Mul => Some(Value::F64((as_f64(&left)? * as_f64(&right)?).into())),
        BinaryOp::Div => {
            let rhs = as_f64(&right)?;
            if rhs == 0.0 {
                return None;
            }
            Some(Value::F64((as_f64(&left)? / rhs).into()))
        }
        BinaryOp::Mod => {
            let rhs = as_f64(&right)?;
            if rhs == 0.0 {
                return None;
            }
            Some(Value::F64((as_f64(&left)? % rhs).into()))
        }
        BinaryOp::Concat => match (left, right) {
            (Value::String(a), Value::String(b)) => Some(Value::String(format!("{a}{b}"))),
            _ => None,
        },
        BinaryOp::And => Some(Value::Bool(value_truthy(&left) && value_truthy(&right))),
        BinaryOp::Or => Some(Value::Bool(value_truthy(&left) || value_truthy(&right))),
        BinaryOp::Eq => Some(Value::Bool(left == right)),
        BinaryOp::NotEq => Some(Value::Bool(left != right)),
        BinaryOp::Lt => Some(Value::Bool(left < right)),
        BinaryOp::Lte => Some(Value::Bool(left <= right)),
        BinaryOp::Gt => Some(Value::Bool(left > right)),
        BinaryOp::Gte => Some(Value::Bool(left >= right)),
        BinaryOp::In => match right {
            Value::List(items) => Some(Value::Bool(items.into_iter().any(|item| item == left))),
            _ => None,
        },
    }
}

fn evaluate_function<T: ObjectAccess + ?Sized>(
    value: &T,
    name: &str,
    args: &[FunctionArg],
) -> Option<Value> {
    let evaluated = args
        .iter()
        .map(|arg| match arg {
            FunctionArg::Expr(expr) => evaluate_expr(value, expr),
            FunctionArg::Wildcard => Some(Value::Bool(true)),
        })
        .collect::<Vec<_>>();
    match name.to_ascii_lowercase().as_str() {
        "coalesce" => evaluated.into_iter().flatten().find(|v| !v.is_nullish()),
        "lower" => evaluated
            .first()
            .and_then(|v| v.as_ref())
            .and_then(Value::as_str)
            .map(|s| Value::String(s.to_ascii_lowercase())),
        "upper" => evaluated
            .first()
            .and_then(|v| v.as_ref())
            .and_then(Value::as_str)
            .map(|s| Value::String(s.to_ascii_uppercase())),
        "count" => {
            if args.iter().any(|arg| matches!(arg, FunctionArg::Wildcard)) {
                Some(Value::I64(1))
            } else {
                let count = evaluated
                    .into_iter()
                    .flatten()
                    .filter(|v| !v.is_nullish())
                    .count();
                Some(Value::I64(count as i64))
            }
        }
        _ => None,
    }
}

fn like_match(input: &str, pattern: &str, case_insensitive: bool) -> bool {
    like_match_inner(input, pattern, case_insensitive)
}

fn like_match_inner(input: &str, pattern: &str, case_insensitive: bool) -> bool {
    // Literal segments are O(input + pattern): anchors are checked once and
    // unanchored segments use KMP over disjoint ranges. Segments containing `_`
    // use multiword Shift-And in O(input * ceil(pattern / 64)).
    if !pattern.contains('%') {
        return match_like_segment_at(input, 0, pattern, case_insensitive) == Some(input.len());
    }

    let segments = pattern.split('%').collect::<Vec<_>>();
    let mut first_unanchored = 0;
    let mut last_unanchored = segments.len();
    let mut cursor = 0;

    if !pattern.starts_with('%') {
        let Some(end) = match_like_segment_at(input, 0, segments[0], case_insensitive) else {
            return false;
        };
        cursor = end;
        first_unanchored = 1;
    }

    let suffix_start = if !pattern.ends_with('%') {
        last_unanchored -= 1;
        let Some(start) =
            match_like_segment_at_end(input, segments[last_unanchored], case_insensitive)
        else {
            return false;
        };
        start
    } else {
        input.len()
    };

    if cursor > suffix_start {
        return false;
    }

    for segment in &segments[first_unanchored..last_unanchored] {
        if segment.is_empty() {
            continue;
        }
        let Some((_, relative_end)) =
            find_like_segment(&input[cursor..suffix_start], segment, case_insensitive)
        else {
            return false;
        };
        cursor += relative_end;
    }

    cursor <= suffix_start
}

fn match_like_segment_at(
    input: &str,
    start: usize,
    segment: &str,
    case_insensitive: bool,
) -> Option<usize> {
    let mut input_chars = input[start..].char_indices();
    let mut end = start;
    for pattern_char in segment.chars() {
        let (relative_index, input_char) = input_chars.next()?;
        if pattern_char != '_' && !like_chars_equal(input_char, pattern_char, case_insensitive) {
            return None;
        }
        end = start + relative_index + input_char.len_utf8();
    }
    Some(end)
}

fn match_like_segment_at_end(input: &str, segment: &str, case_insensitive: bool) -> Option<usize> {
    let mut input_chars = input.char_indices().rev();
    let mut start = input.len();
    for pattern_char in segment.chars().rev() {
        let (input_index, input_char) = input_chars.next()?;
        if pattern_char != '_' && !like_chars_equal(input_char, pattern_char, case_insensitive) {
            return None;
        }
        start = input_index;
    }
    Some(start)
}

fn find_like_segment(input: &str, segment: &str, case_insensitive: bool) -> Option<(usize, usize)> {
    if segment.contains('_') {
        return find_wildcard_like_segment(input, segment, case_insensitive);
    }

    find_literal_like_segment(input, segment, case_insensitive)
}

fn find_wildcard_like_segment(
    input: &str,
    segment: &str,
    case_insensitive: bool,
) -> Option<(usize, usize)> {
    const WORD_BITS: usize = u64::BITS as usize;

    let pattern_len = segment.chars().count();
    if pattern_len == 0 {
        return Some((0, 0));
    }

    // Shift-And NFA state is split into machine words. Literal masks are kept
    // per word so storage is O(pattern), even when every scalar is distinct.
    // Matching is O(input * ceil(pattern / 64)) with no length cap.
    let word_count = pattern_len.div_ceil(WORD_BITS);
    let mut literal_masks = (0..word_count)
        .map(|_| HashMap::<char, u64>::new())
        .collect::<Vec<_>>();
    let mut wildcard_masks = vec![0u64; word_count];

    for (index, pattern_char) in segment.chars().enumerate() {
        let word = index / WORD_BITS;
        let bit = 1u64 << (index % WORD_BITS);
        if pattern_char == '_' {
            wildcard_masks[word] |= bit;
        } else {
            let key = normalize_like_char(pattern_char, case_insensitive);
            *literal_masks[word].entry(key).or_default() |= bit;
        }
    }

    let match_word = (pattern_len - 1) / WORD_BITS;
    let match_bit = 1u64 << ((pattern_len - 1) % WORD_BITS);
    let mut state = vec![0u64; word_count];

    for (input_index, input_char) in input.char_indices() {
        let key = normalize_like_char(input_char, case_insensitive);
        let mut carry = 1u64;
        for word in 0..word_count {
            let previous = state[word];
            let shifted = (previous << 1) | carry;
            carry = previous >> (WORD_BITS - 1);
            let literal_mask = literal_masks[word].get(&key).copied().unwrap_or(0);
            state[word] = shifted & (literal_mask | wildcard_masks[word]);
        }

        if state[match_word] & match_bit != 0 {
            let end = input_index + input_char.len_utf8();
            let start = input[..end]
                .char_indices()
                .rev()
                .nth(pattern_len - 1)
                .map(|(index, _)| index)
                .expect("a complete match contains every pattern scalar");
            return Some((start, end));
        }
    }

    None
}

fn find_literal_like_segment(
    input: &str,
    segment: &str,
    case_insensitive: bool,
) -> Option<(usize, usize)> {
    let pattern = segment.chars().collect::<Vec<_>>();
    if pattern.is_empty() {
        return Some((0, 0));
    }

    // KMP makes literal segments linear even for adversarial near matches such
    // as `%aaaa...b` against a long run of `a` characters.
    let mut failure = vec![0; pattern.len()];
    let mut prefix_len = 0;
    for index in 1..pattern.len() {
        while prefix_len > 0
            && !like_chars_equal(pattern[index], pattern[prefix_len], case_insensitive)
        {
            prefix_len = failure[prefix_len - 1];
        }
        if like_chars_equal(pattern[index], pattern[prefix_len], case_insensitive) {
            prefix_len += 1;
            failure[index] = prefix_len;
        }
    }

    let mut matched = 0;
    for (input_index, input_char) in input.char_indices() {
        while matched > 0 && !like_chars_equal(input_char, pattern[matched], case_insensitive) {
            matched = failure[matched - 1];
        }
        if like_chars_equal(input_char, pattern[matched], case_insensitive) {
            matched += 1;
        }
        if matched == pattern.len() {
            let end = input_index + input_char.len_utf8();
            let start = input[..end]
                .char_indices()
                .rev()
                .nth(pattern.len() - 1)
                .map(|(index, _)| index)
                .expect("a complete match contains every pattern scalar");
            return Some((start, end));
        }
    }

    None
}

fn like_chars_equal(left: char, right: char, case_insensitive: bool) -> bool {
    normalize_like_char(left, case_insensitive) == normalize_like_char(right, case_insensitive)
}

fn normalize_like_char(value: char, case_insensitive: bool) -> char {
    if case_insensitive {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn similar_to_match(input: &str, pattern: &str, case_insensitive: bool) -> bool {
    like_match(input, pattern, case_insensitive)
}

fn regex_match(input: &str, pattern: &str, case_insensitive: bool) -> Option<bool> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .ok()?;
    Some(re.is_match(input))
}

fn value_truthy(value: &Value) -> bool {
    match value {
        Value::Void | Value::Null => false,
        Value::Bool(v) => *v,
        Value::I8(v) => *v != 0,
        Value::I16(v) => *v != 0,
        Value::I32(v) => *v != 0,
        Value::I64(v) => *v != 0,
        Value::I128(v) => *v != 0,
        Value::U8(v) => *v != 0,
        Value::U16(v) => *v != 0,
        Value::U32(v) => *v != 0,
        Value::U64(v) => *v != 0,
        Value::U128(v) => *v != 0,
        Value::F32(v) => v.into_inner() != 0.0,
        Value::F64(v) => v.into_inner() != 0.0,
        Value::String(v) => !v.is_empty(),
        Value::Bytes(v) => !v.is_empty(),
        Value::List(v) => !v.is_empty(),
        Value::Map(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        _ => true,
    }
}

fn map_value_at_path_ref<'a>(
    object: &'a BTreeMap<String, Value>,
    path: &FieldPath,
) -> Option<ValueRef<'a>> {
    let mut current = path.segments().first().and_then(|first| match first {
        PathSegment::Field(field) => object_field_with_alias_fallback(object, field),
        PathSegment::Index(_) => None,
    })?;

    for segment in path.segments().iter().skip(1) {
        current = match segment {
            PathSegment::Field(field) => value_field_with_alias_fallback(current, field)?,
            PathSegment::Index(index) => current.get_index(*index)?,
        };
    }

    Some(ValueRef::Ref(current))
}

fn value_field_with_alias_fallback<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => object_field_with_alias_fallback(object, field),
        _ => None,
    }
}

fn object_field_with_alias_fallback<'a>(
    object: &'a BTreeMap<String, Value>,
    field: &str,
) -> Option<&'a Value> {
    if let Some(value) = object.get(field) {
        return Some(value);
    }
    let wanted_plain = field.rsplit(':').next().unwrap_or(field);
    let mut matching = object.iter().filter(|(key, _)| {
        key.rsplit(':')
            .next()
            .is_some_and(|plain| plain == wanted_plain)
    });
    let (_, value) = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(value)
}

pub fn first_indexable_equality_predicate(predicate: &Expr) -> Option<(String, Value)> {
    match predicate {
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Operand(Operand::Field(path)), Expr::Operand(Operand::Literal(value)))
            | (Expr::Operand(Operand::Literal(value)), Expr::Operand(Operand::Field(path))) => {
                match path.segments().first() {
                    Some(PathSegment::Field(field)) if path.segments().len() == 1 => {
                        Some((field.clone(), value.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        },
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => first_indexable_equality_predicate(left)
            .or_else(|| first_indexable_equality_predicate(right)),
        _ => None,
    }
}

pub fn touched_collections(batch: &Batch) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for op in &batch.operations {
        match op {
            BatchOperation::Upsert { collection, .. }
            | BatchOperation::Create { collection, .. }
            | BatchOperation::DeleteById { collection, .. }
            | BatchOperation::DeleteByIds { collection, .. }
            | BatchOperation::Update { collection, .. }
            | BatchOperation::Delete { collection, .. } => {
                out.insert(collection.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_filters_and_projects() {
        let mut a = Object::new();
        a.insert("kind", Value::String("music".into()));
        a.insert("score", Value::I64(9));

        let mut b = Object::new();
        b.insert("kind", Value::String("video".into()));
        b.insert("score", Value::I64(2));

        let query = SelectQuery::new()
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "kind",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "music".into(),
                )))),
            })
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "score",
                ])))),
                alias: Some("s".into()),
                wildcard: None,
            }]);

        let out = execute_query(&query, vec![a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("s"), Some(&Value::I64(9)));
    }

    #[test]
    fn project_object_flattens_qualified_wildcard() {
        let mut child = Object::new();
        child.insert("id", Value::String("child-1".to_string()));
        child.insert("title", Value::String("Child".to_string()));

        let mut row = Object::new();
        row.insert("child", Value::Object(child));
        row.insert("order", Value::U64(7));

        let out = project_object(
            &row,
            &[
                QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "child",
                    ])))),
                    alias: None,
                    wildcard: Some(FieldPath::from_fields(["child"])),
                },
                QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "order",
                    ])))),
                    alias: Some("directory_order".to_string()),
                    wildcard: None,
                },
            ],
        );

        assert_eq!(out.get("id"), Some(&Value::String("child-1".to_string())));
        assert_eq!(out.get("title"), Some(&Value::String("Child".to_string())));
        assert_eq!(out.get("directory_order"), Some(&Value::U64(7)));
        assert!(!out.contains_key("child"));
    }

    #[test]
    fn project_object_flattens_current_row_wildcard() {
        let mut row = Object::new();
        row.insert("id", Value::String("entity-1".to_string()));
        row.insert("title", Value::String("Entity".to_string()));

        let out = project_object(
            &row,
            &[QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["d"])))),
                alias: None,
                wildcard: Some(FieldPath::new()),
            }],
        );

        assert_eq!(out, row);
    }

    #[test]
    fn query_machine_streaming_no_sort() {
        let query = SelectQuery::new().with_limit(1);
        let mut machine = QueryStateMachine::new(query);

        let mut a = Object::new();
        a.insert("id", Value::String("a".into()));
        machine.push_row(a);

        let mut b = Object::new();
        b.insert("id", Value::String("b".into()));
        machine.push_row(b);

        machine.finish();
        let out = machine.drain_ready();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("id"), Some(&Value::String("a".into())));
    }

    #[test]
    fn execute_query_sorts_by_expression() {
        let mut a = Object::new();
        a.insert("id", Value::String("a".into()));
        a.insert("score", Value::I64(3));

        let mut b = Object::new();
        b.insert("id", Value::String("b".into()));
        b.insert("score", Value::I64(10));

        let mut c = Object::new();
        c.insert("id", Value::String("c".into()));
        c.insert("score", Value::I64(6));

        let query = SelectQuery::new().with_order_by(vec![OrderBy {
            expr: Expr::Binary {
                op: BinaryOp::Mul,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "score",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::I64(-1)))),
            },
            direction: SortDirection::Asc,
        }]);

        let out = execute_query(&query, vec![a, b, c]);
        assert_eq!(out[0].get("id"), Some(&Value::String("b".into())));
        assert_eq!(out[1].get("id"), Some(&Value::String("c".into())));
        assert_eq!(out[2].get("id"), Some(&Value::String("a".into())));
    }

    #[test]
    fn update_query_with_expr() {
        let mut rows = vec![Entity {
            id: "1".into(),
            collection: "items".into(),
            object: {
                let mut o = Object::new();
                o.insert("score", Value::I64(2));
                o
            },
        }];

        let query = UpdateQuery::new().set(
            FieldPath::from_fields(["score"]),
            Expr::Binary {
                op: BinaryOp::Add,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "score",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::I64(3)))),
            },
        );

        let stats = apply_update(&query, &mut rows).unwrap();
        assert_eq!(stats.affected, 1);
        assert_eq!(rows[0].object.get("score"), Some(&Value::F64(5.0.into())));
    }

    #[test]
    fn update_assignments_read_the_original_row() {
        let mut object = Object::new();
        object.insert("a", Value::I64(1));
        object.insert("b", Value::I64(2));
        let mut rows = vec![Entity {
            id: "1".into(),
            collection: "items".into(),
            object,
        }];

        let query = UpdateQuery::new()
            .set(
                FieldPath::from_fields(["a"]),
                Expr::Operand(Operand::Field(FieldPath::from_fields(["b"]))),
            )
            .set(
                FieldPath::from_fields(["b"]),
                Expr::Operand(Operand::Field(FieldPath::from_fields(["a"]))),
            );

        let stats = apply_update(&query, &mut rows).unwrap();
        assert_eq!(stats.affected, 1);
        assert_eq!(rows[0].object.get("a"), Some(&Value::I64(2)));
        assert_eq!(rows[0].object.get("b"), Some(&Value::I64(1)));
    }

    #[test]
    fn failed_update_assignment_does_not_partially_modify_row() {
        let mut object = Object::new();
        object.insert("a", Value::I64(1));
        let mut rows = vec![Entity {
            id: "1".into(),
            collection: "items".into(),
            object,
        }];

        let query = UpdateQuery::new()
            .set(
                FieldPath::from_fields(["a"]),
                Expr::Operand(Operand::Literal(Value::I64(9))),
            )
            .set(
                FieldPath::from_fields(["b"]),
                Expr::Operand(Operand::Field(FieldPath::from_fields(["missing"]))),
            );

        assert!(apply_update(&query, &mut rows).is_err());
        assert_eq!(rows[0].object.get("a"), Some(&Value::I64(1)));
        assert!(!rows[0].object.contains_key("b"));
    }

    #[test]
    fn failed_update_rolls_back_rows_staged_before_the_error() {
        let mut first = Object::new();
        first.insert("value", Value::I64(1));
        first.insert("source", Value::I64(10));
        let mut second = Object::new();
        second.insert("value", Value::I64(2));
        let mut rows = vec![
            Entity {
                id: "1".into(),
                collection: "items".into(),
                object: first,
            },
            Entity {
                id: "2".into(),
                collection: "items".into(),
                object: second,
            },
        ];
        let before = rows.clone();
        let query = UpdateQuery::new().set(
            FieldPath::from_fields(["value"]),
            Expr::Operand(Operand::Field(FieldPath::from_fields(["source"]))),
        );

        assert!(apply_update_with_returning(&query, &mut rows).is_err());
        assert_eq!(rows, before);
    }

    #[test]
    fn invalid_mutation_limits_fail_closed() {
        let mut object = Object::new();
        object.insert("a", Value::I64(1));
        let entity = Entity {
            id: "1".into(),
            collection: "items".into(),
            object,
        };
        let invalid_limit = Expr::Operand(Operand::Literal(Value::String("all".into())));

        let mut update_rows = vec![entity.clone()];
        let update = UpdateQuery::new()
            .set(
                FieldPath::from_fields(["a"]),
                Expr::Operand(Operand::Literal(Value::I64(9))),
            )
            .with_limit(invalid_limit.clone());
        assert!(apply_update(&update, &mut update_rows).is_err());
        assert_eq!(update_rows, vec![entity.clone()]);

        let delete = DeleteQuery::new().with_limit(invalid_limit);
        let (remaining, deleted) = apply_delete(&delete, vec![entity.clone()]);
        assert_eq!(deleted, 0);
        assert_eq!(remaining, vec![entity]);
    }

    #[test]
    fn delete_with_remaining_preserves_limit_order_and_compatibility() {
        let entities = ["a", "b", "c"]
            .into_iter()
            .map(|id| {
                let mut object = Object::new();
                object.insert("id", Value::String(id.to_string()));
                Entity {
                    id: id.to_string(),
                    collection: "items".into(),
                    object,
                }
            })
            .collect::<Vec<_>>();
        let query = DeleteQuery::new()
            .with_limit(2)
            .with_returning(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                alias: None,
                wildcard: None,
            }]);

        let (remaining, result) = apply_delete_with_remaining(&query, entities.clone());
        assert_eq!(
            remaining
                .iter()
                .map(|entity| entity.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c"],
        );
        assert_eq!(result.deleted, 2);
        assert_eq!(
            result
                .returning
                .iter()
                .filter_map(|row| row.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["a", "b"],
        );

        let zero_query = query.clone().with_limit(0);
        let (zero_remaining, zero_result) =
            apply_delete_with_remaining(&zero_query, entities.clone());
        assert_eq!(zero_remaining, entities);
        assert_eq!(zero_result.deleted, 0);
        assert!(zero_result.returning.is_empty());

        let (legacy_remaining, legacy_deleted) = apply_delete(&query, entities.clone());
        assert_eq!(legacy_remaining, remaining);
        assert_eq!(legacy_deleted, result.deleted);
        assert_eq!(apply_delete_with_returning(&query, entities), result);
    }

    #[test]
    fn like_matches_unicode_scalars_and_ascii_case_insensitively() {
        assert!(like_match("a🦀界", "a_界", false));
        assert!(!like_match("a🦀界", "a__界", false));
        assert!(like_match("prefix界", "%界", false));
        assert!(like_match("ab🦀cd界ef", "a%_cd%ef", false));
        assert!(like_match("RuSt", "r_st", true));
        assert!(!like_match("RuSt", "r_st", false));
    }

    #[test]
    fn like_handles_long_adversarial_patterns_without_recursion() {
        let input = "🦀".repeat(20_000);
        let scalar_pattern = "_".repeat(20_000);
        assert!(like_match(&input, &scalar_pattern, false));

        let wildcard_pattern = "%".repeat(50_000);
        assert!(like_match("", &wildcard_pattern, false));

        let multiword_input = format!("{}界{}", "🦀".repeat(64), "🦀".repeat(64));
        let multiword_pattern = format!("%{}界{}%", "_".repeat(64), "_".repeat(64));
        assert!(like_match(&multiword_input, &multiword_pattern, false,));
    }

    #[test]
    fn like_literal_segment_near_match_is_non_quadratic() {
        let input = "a".repeat(100_000);
        let literal = format!("{}b", "a".repeat(10_000));
        assert!(!like_match(&input, &format!("%{literal}"), false));
        assert!(!like_match(&input, &format!("%{literal}%"), false));

        let wildcard_literal = format!("_{}b", "a".repeat(10_000));
        assert!(!like_match(&input, &format!("%{wildcard_literal}%"), false,));
    }

    #[test]
    fn like_matches_exhaustive_reference_cases() {
        fn enumerate(alphabet: &[char], max_len: usize) -> Vec<String> {
            let mut all = vec![String::new()];
            let mut frontier = vec![String::new()];
            for _ in 0..max_len {
                let mut next = Vec::new();
                for prefix in &frontier {
                    for value in alphabet {
                        let mut item = prefix.clone();
                        item.push(*value);
                        next.push(item);
                    }
                }
                all.extend(next.iter().cloned());
                frontier = next;
            }
            all
        }

        fn reference(input: &str, pattern: &str, case_insensitive: bool) -> bool {
            let input = input.chars().collect::<Vec<_>>();
            let pattern = pattern.chars().collect::<Vec<_>>();
            let mut matches = vec![vec![false; pattern.len() + 1]; input.len() + 1];
            matches[input.len()][pattern.len()] = true;

            for pattern_index in (0..pattern.len()).rev() {
                if pattern[pattern_index] == '%' {
                    matches[input.len()][pattern_index] = matches[input.len()][pattern_index + 1];
                    for input_index in (0..input.len()).rev() {
                        matches[input_index][pattern_index] = matches[input_index]
                            [pattern_index + 1]
                            || matches[input_index + 1][pattern_index];
                    }
                } else {
                    for input_index in (0..input.len()).rev() {
                        let scalar_matches = pattern[pattern_index] == '_'
                            || like_chars_equal(
                                input[input_index],
                                pattern[pattern_index],
                                case_insensitive,
                            );
                        matches[input_index][pattern_index] =
                            scalar_matches && matches[input_index + 1][pattern_index + 1];
                    }
                }
            }

            matches[0][0]
        }

        let inputs = enumerate(&['a', 'B', '🦀'], 4);
        let patterns = enumerate(&['a', 'B', '🦀', '_', '%'], 4);
        for case_insensitive in [false, true] {
            for input in &inputs {
                for pattern in &patterns {
                    assert_eq!(
                        like_match(input, pattern, case_insensitive),
                        reference(input, pattern, case_insensitive),
                        "input={input:?}, pattern={pattern:?}, case_insensitive={case_insensitive}",
                    );
                }
            }
        }
    }

    #[test]
    fn batch_is_atomic_pure_function() {
        let mut dataset = Dataset::new();
        dataset
            .entry("items".into())
            .or_default()
            .insert("a".into(), {
                let mut o = Object::new();
                o.insert("v", Value::I64(1));
                o
            });

        let batch = Batch::new().with_op(BatchOperation::Update {
            collection: "items".into(),
            query: UpdateQuery::new().set(
                FieldPath::from_fields(["v"]),
                Expr::Operand(Operand::Literal(Value::I64(10))),
            ),
        });

        let out = execute_batch(&dataset, &batch).unwrap();
        assert_eq!(
            out.dataset
                .get("items")
                .and_then(|c| c.get("a"))
                .and_then(|o| o.get("v")),
            Some(&Value::I64(10))
        );
        assert_eq!(
            dataset
                .get("items")
                .and_then(|c| c.get("a"))
                .and_then(|o| o.get("v")),
            Some(&Value::I64(1))
        );
    }
}
