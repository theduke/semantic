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

pub type QueryInput = TextQueryInput;

pub use prql::*;
pub use sql::*;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
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

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

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
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum UnaryOp {
    Not,
    Neg,
}

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
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Predicate {
    Compare {
        op: CompareOp,
        left: Operand,
        right: Operand,
    },
    Expr(Expr),
    Exists(FieldPath),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct QueryField {
    pub path: FieldPath,
    pub alias: Option<String>,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub path: FieldPath,
    pub direction: SortDirection,
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

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum JoinCondition {
    OnPredicate(Predicate),
    UsingFields { left: FieldPath, right: FieldPath },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct JoinQuery {
    pub source: String,
    pub alias: Option<String>,
    pub join_type: JoinType,
    pub condition: JoinCondition,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct SelectQuery {
    pub collection: Option<String>,
    pub source_alias: Option<String>,
    pub joins: Vec<JoinQuery>,
    pub predicate: Option<Predicate>,
    pub projection: Vec<QueryField>,
    pub order_by: Vec<OrderBy>,
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Query {
    Select(SelectQuery),
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

impl From<DeleteQuery> for Query {
    fn from(value: DeleteQuery) -> Self {
        Self::Delete(value)
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
            order_by: Vec::new(),
            offset: 0,
            limit: None,
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

    pub fn with_predicate(mut self, predicate: Predicate) -> Self {
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

    pub fn with_order_by(mut self, order_by: Vec<OrderBy>) -> Self {
        self.order_by = order_by;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
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
        if self.consumed_offset < self.query.offset {
            self.consumed_offset += 1;
            return;
        }

        if let Some(limit) = self.query.limit {
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
pub struct UpdateQuery {
    pub collection: Option<String>,
    pub predicate: Option<Predicate>,
    pub assignments: Vec<Assignment>,
    pub limit: Option<usize>,
    pub returning: Vec<QueryField>,
}

impl UpdateQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            predicate: None,
            assignments: Vec::new(),
            limit: None,
            returning: Vec::new(),
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

    pub fn with_predicate(mut self, predicate: Predicate) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn set(mut self, path: FieldPath, value: Expr) -> Self {
        self.assignments.push(Assignment { path, value });
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_returning(mut self, projection: Vec<QueryField>) -> Self {
        self.returning = projection;
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
    pub predicate: Option<Predicate>,
    pub limit: Option<usize>,
    pub returning: Vec<QueryField>,
}

impl DeleteQuery {
    pub fn new() -> Self {
        Self {
            collection: None,
            predicate: None,
            limit: None,
            returning: Vec::new(),
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

    pub fn with_predicate(mut self, predicate: Predicate) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_returning(mut self, projection: Vec<QueryField>) -> Self {
        self.returning = projection;
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

    for entity in entities.iter_mut() {
        if !row_matches(&entity.object, &query.predicate) {
            continue;
        }

        if let Some(limit) = query.limit {
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

    for entity in entities {
        if row_matches(&entity.object, &query.predicate)
            && query.limit.map(|limit| deleted < limit).unwrap_or(true)
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

pub fn evaluate_predicate<T: ObjectAccess + ?Sized>(value: &T, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::Compare { op, left, right } => {
            let left = resolve_operand(value, left);
            let right = resolve_operand(value, right);
            compare_values(*op, left, right)
        }
        Predicate::Expr(expr) => evaluate_expr(value, expr)
            .as_ref()
            .is_some_and(value_truthy),
        Predicate::Exists(path) => value.value_at_path_ref(path).is_some(),
        Predicate::And(items) => items.iter().all(|item| evaluate_predicate(value, item)),
        Predicate::Or(items) => items.iter().any(|item| evaluate_predicate(value, item)),
        Predicate::Not(item) => !evaluate_predicate(value, item),
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
    }
}

pub fn project_object<T: ObjectAccess + ?Sized>(value: &T, projection: &[QueryField]) -> Object {
    let mut out = Object::new();
    for project in projection {
        let Some(v) = value.value_at_path_ref(&project.path) else {
            continue;
        };

        let key = project
            .alias
            .clone()
            .unwrap_or_else(|| infer_project_key(&project.path));
        out.insert(key, v.into_owned());
    }

    out
}

pub fn row_matches<T: ObjectAccess + ?Sized>(row: &T, predicate: &Option<Predicate>) -> bool {
    match predicate {
        Some(predicate) => evaluate_predicate(row, predicate),
        None => true,
    }
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
        let av = a.value_at_path_ref(&order.path);
        let bv = b.value_at_path_ref(&order.path);

        let ord = match (av.as_ref(), bv.as_ref()) {
            (Some(av), Some(bv)) => av.clone().into_owned().cmp(&bv.clone().into_owned()),
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

fn resolve_operand<'a, T: ObjectAccess + ?Sized>(
    value: &'a T,
    operand: &'a Operand,
) -> Option<ValueRef<'a>> {
    match operand {
        Operand::Field(path) => value.value_at_path_ref(path),
        Operand::Literal(value) => Some(ValueRef::Ref(value)),
    }
}

fn compare_values(op: CompareOp, left: Option<ValueRef<'_>>, right: Option<ValueRef<'_>>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let left = left.into_owned();
    let right = right.into_owned();

    match op {
        CompareOp::Eq => left == right,
        CompareOp::NotEq => left != right,
        CompareOp::Lt => left < right,
        CompareOp::Lte => left <= right,
        CompareOp::Gt => left > right,
        CompareOp::Gte => left >= right,
    }
}

fn infer_project_key(path: &FieldPath) -> String {
    for segment in path.segments().iter().rev() {
        if let PathSegment::Field(name) = segment {
            return name.clone();
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
    }
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

pub fn first_indexable_equality_predicate(predicate: &Predicate) -> Option<(String, Value)> {
    match predicate {
        Predicate::Compare {
            op: CompareOp::Eq,
            left,
            right,
        } => match (left, right) {
            (Operand::Field(path), Operand::Literal(value))
            | (Operand::Literal(value), Operand::Field(path)) => match path.segments().first() {
                Some(PathSegment::Field(field)) if path.segments().len() == 1 => {
                    Some((field.clone(), value.clone()))
                }
                _ => None,
            },
            _ => None,
        },
        Predicate::And(items) => items.iter().find_map(first_indexable_equality_predicate),
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
            .with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["kind"])),
                right: Operand::Literal(Value::String("music".into())),
            })
            .with_projection(vec![QueryField {
                path: FieldPath::from_fields(["score"]),
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
