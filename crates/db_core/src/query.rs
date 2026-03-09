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

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FieldFormat {
    Qualified,
    Underscore,
    Plain,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextQueryInput {
    Ast(Query),
    Text {
        format: TextQueryFormat,
        query: String,
    },
}

impl TextQueryInput {
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

impl From<public_query::FieldFormat> for FieldFormat {
    fn from(value: public_query::FieldFormat) -> Self {
        match value {
            public_query::FieldFormat::Qualified => Self::Qualified,
            public_query::FieldFormat::Underscore => Self::Underscore,
            public_query::FieldFormat::Plain => Self::Plain,
        }
    }
}

impl From<FieldFormat> for public_query::FieldFormat {
    fn from(value: FieldFormat) -> Self {
        match value {
            FieldFormat::Qualified => Self::Qualified,
            FieldFormat::Underscore => Self::Underscore,
            FieldFormat::Plain => Self::Plain,
        }
    }
}

impl From<public_query::QueryInput> for TextQueryInput {
    fn from(value: public_query::QueryInput) -> Self {
        match value {
            public_query::QueryInput::Ast(query) => Self::Ast(query.into()),
            public_query::QueryInput::Text { format, query } => Self::Text {
                format: format.into(),
                query,
            },
        }
    }
}

pub type QueryInput = TextQueryInput;

pub use prql::*;
pub use sql::*;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use regex::RegexBuilder;
use semantic_data::query as public_query;
use semantic_data::query::{
    AggregateOp, BinaryOp, JoinType, PatternMatchKind, SortDirection, UnaryOp,
};
use semantic_data::value::{FieldPath, Object, PathSegment, Value, ValueRef};

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
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
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
                coll.insert(id.clone(), object.clone());
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

                    let result = apply_update(query, &mut entities)?;
                    stats.updated += result.affected;

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
    let mut matched = 0usize;
    let mut affected = 0usize;
    let mut returning = Vec::new();
    let limit = query.limit.as_ref().and_then(evaluate_usize_expr);

    for entity in entities.iter_mut() {
        if !row_matches(&entity.object, &query.predicate) {
            continue;
        }

        if let Some(limit) = limit {
            if matched >= limit {
                break;
            }
        }

        matched += 1;
        let before = entity.object.clone();

        for assignment in &query.assignments {
            let value = evaluate_expr(&entity.object, &assignment.value)
                .ok_or_else(|| CoreError::new("failed to evaluate assignment expression"))?;
            set_value_at_path(&mut entity.object, &assignment.path, value)?;
        }

        if !query.returning.is_empty() {
            returning.push(project_object(&entity.object, &query.returning));
        }

        if entity.object != before {
            affected += 1;
        }
    }

    Ok(UpdateResult {
        stats: MutationStats { matched, affected },
        returning,
    })
}

pub fn apply_delete(query: &DeleteQuery, entities: Vec<Entity>) -> (Vec<Entity>, usize) {
    let result = apply_delete_plan(query, entities);
    (result.remaining, result.deleted)
}

struct DeletePlanResult {
    remaining: Vec<Entity>,
    deleted: usize,
    returning: Vec<Object>,
}

pub fn apply_delete_with_returning(query: &DeleteQuery, entities: Vec<Entity>) -> DeleteResult {
    let DeletePlanResult {
        deleted, returning, ..
    } = apply_delete_plan(query, entities);
    DeleteResult { deleted, returning }
}

fn apply_delete_plan(query: &DeleteQuery, entities: Vec<Entity>) -> DeletePlanResult {
    let mut deleted = 0usize;
    let mut remaining = Vec::with_capacity(entities.len());
    let mut returning = Vec::new();
    let limit = query.limit.as_ref().and_then(evaluate_usize_expr);

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
    let (input, pattern) = if case_insensitive {
        (input.to_ascii_lowercase(), pattern.to_ascii_lowercase())
    } else {
        (input.to_string(), pattern.to_string())
    };
    like_match_inner(input.as_bytes(), pattern.as_bytes())
}

fn like_match_inner(input: &[u8], pattern: &[u8]) -> bool {
    if pattern.is_empty() {
        return input.is_empty();
    }
    match pattern[0] {
        b'%' => {
            for i in 0..=input.len() {
                if like_match_inner(&input[i..], &pattern[1..]) {
                    return true;
                }
            }
            false
        }
        b'_' => {
            if input.is_empty() {
                false
            } else {
                like_match_inner(&input[1..], &pattern[1..])
            }
        }
        c => {
            if input.first().copied() == Some(c) {
                like_match_inner(&input[1..], &pattern[1..])
            } else {
                false
            }
        }
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
        PathSegment::Field(field) => object.get(field),
        PathSegment::Index(_) => None,
    })?;

    for segment in path.segments().iter().skip(1) {
        current = match segment {
            PathSegment::Field(field) => current.get_field(field)?,
            PathSegment::Index(index) => current.get_index(*index)?,
        };
    }

    Some(ValueRef::Ref(current))
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
            }]);

        let out = execute_query(&query, vec![a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("s"), Some(&Value::I64(9)));
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
