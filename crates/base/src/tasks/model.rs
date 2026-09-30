use super::schema::*;
use crate::{
    content::{ATTR_MAIN_CONTENT, MainContent},
    domain_support::{optional, read},
};
use semantic_data::{
    attr::{ATTR_CREATED_AT, ATTR_PARENT, ATTR_TITLE, ATTR_UPDATED_AT},
    value::{Date, DateTime, FromValue, IntoValue, Object, SemanticType, Value},
};
use semantic_rpc_core::RpcError;
#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[semantic(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    #[default]
    Todo,
    InProgress,
    Blocked,
    Done,
    Canceled,
}
impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[semantic(rename_all = "snake_case")]
pub enum TaskPriority {
    #[default]
    None,
    Low,
    Medium,
    High,
    Urgent,
}
impl TaskPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub main_content: MainContent,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub progress: u64,
    pub due_date: Option<DueDate>,
    pub parent: Option<String>,
    pub archived: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
impl Task {
    pub fn from_object(o: &Object) -> Result<Self, RpcError> {
        if o.get("type").and_then(Value::as_str) != Some(CLASS_ID) {
            return Err(crate::domain_support::error("Entity is not a task"));
        }
        Ok(Self {
            id: read(o, "id")?,
            title: read(o, ATTR_TITLE)?,
            main_content: read(o, ATTR_MAIN_CONTENT)?,
            status: read(o, ATTR_STATUS)?,
            priority: read(o, ATTR_PRIORITY)?,
            progress: read(o, ATTR_PROGRESS)?,
            due_date: optional(o, ATTR_DUE_DATE)?,
            parent: optional(o, ATTR_PARENT)?,
            archived: read(o, ATTR_ARCHIVED)?,
            created_at: read(o, ATTR_CREATED_AT)?,
            updated_at: read(o, ATTR_UPDATED_AT)?,
        })
    }
    pub fn to_object(&self) -> Object {
        let mut o = Object::new();
        o.insert("id", self.id.clone().into_value());
        o.insert("type", CLASS_ID.to_owned().into_value());
        self.apply(&mut o);
        o.insert(ATTR_CREATED_AT, self.created_at.into_value());
        o
    }
    pub(crate) fn apply(&self, o: &mut Object) {
        o.insert(ATTR_TITLE, self.title.clone().into_value());
        o.insert(ATTR_MAIN_CONTENT, self.main_content.clone().into_value());
        o.insert(ATTR_STATUS, self.status.into_value());
        o.insert(ATTR_PRIORITY, self.priority.into_value());
        o.insert(ATTR_PROGRESS, self.progress.into_value());
        o.insert(ATTR_ARCHIVED, self.archived.into_value());
        o.insert(ATTR_UPDATED_AT, self.updated_at.into_value());
        for (key, value) in [
            (ATTR_PARENT, self.parent.clone().map(IntoValue::into_value)),
            (ATTR_DUE_DATE, self.due_date.map(IntoValue::into_value)),
        ] {
            if let Some(value) = value {
                o.insert(key, value);
            } else {
                o.remove(key);
            }
        }
    }
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct TaskPage {
    pub tasks: Vec<Task>,
    pub total: u64,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct TaskDetail {
    pub task: Task,
    pub children: Vec<Task>,
}

/// A calendar day with explicit RPC and persisted date conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DueDate(pub Date);
impl From<Date> for DueDate {
    fn from(date: Date) -> Self {
        Self(date)
    }
}
impl SemanticType for DueDate {
    fn semantic_type() -> semantic_data::schema::Type {
        crate::schema::common::helpers::date_type()
    }
}
impl IntoValue for DueDate {
    fn into_value(self) -> Value {
        Value::Date(self.0)
    }
}
impl FromValue for DueDate {
    fn from_value(value: Value) -> Result<Self, semantic_data::value::FromValueError> {
        match value {
            Value::Date(date) => Ok(Self(date)),
            other => Err(semantic_data::value::FromValueError::expected(
                "date", &other,
            )),
        }
    }
}
