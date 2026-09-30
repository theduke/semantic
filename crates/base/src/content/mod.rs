//! Embedded main content shared by optional applications.
pub mod schema;
use crate::schema::notes::{ATTR_NOTE_CONTENT, ATTR_NOTE_FORMAT};
pub use schema::ATTR_MAIN_CONTENT;
use semantic_data::{
    attr::{ATTR_CREATED_AT, ATTR_UPDATED_AT},
    value::{DateTime, FromValue, FromValueError, IntoValue, Object, SemanticType, Value},
};
use semantic_rpc_core::RpcError;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MainContent {
    Note {
        format: String,
        body: String,
        created_at: DateTime,
        updated_at: DateTime,
    },
}
impl MainContent {
    pub fn note(body: impl Into<String>) -> Self {
        let now = DateTime::now_utc();
        Self::Note {
            format: "markdown".into(),
            body: body.into(),
            created_at: now,
            updated_at: now,
        }
    }
    pub fn text(body: impl Into<String>) -> Self {
        let mut content = Self::note(body);
        let Self::Note { format, .. } = &mut content;
        *format = "text".into();
        content
    }
    pub fn body(&self) -> &str {
        match self {
            Self::Note { body, .. } => body,
        }
    }
    pub fn format(&self) -> &str {
        match self {
            Self::Note { format, .. } => format,
        }
    }
    pub fn validate(&self) -> Result<(), RpcError> {
        if !matches!(self.format(), "text" | "markdown") {
            return Err(RpcError::new("invalid_content", "Unsupported Note format"));
        }
        Ok(())
    }
    pub fn to_value(&self) -> Value {
        let Self::Note {
            format,
            body,
            created_at,
            updated_at,
        } = self;
        let mut note = Object::new();
        note.insert(ATTR_NOTE_FORMAT, format.clone().into_value());
        note.insert(ATTR_NOTE_CONTENT, body.clone().into_value());
        note.insert(ATTR_CREATED_AT, (*created_at).into_value());
        note.insert(ATTR_UPDATED_AT, (*updated_at).into_value());
        let mut tagged = Object::new();
        tagged.insert("kind", "note".to_owned().into_value());
        tagged.insert("data", Value::Object(note));
        Value::Object(tagged)
    }
}
impl SemanticType for MainContent {
    fn semantic_type() -> semantic_data::schema::Type {
        schema::content_type()
    }
}
impl IntoValue for MainContent {
    fn into_value(self) -> Value {
        self.to_value()
    }
}
impl FromValue for MainContent {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        let Value::Object(tagged) = value else {
            return Err(FromValueError::new("Expected main content object"));
        };
        if tagged.get("kind").and_then(Value::as_str) != Some("note") {
            return Err(FromValueError::new("Unsupported main content variant"));
        }
        let Some(Value::Object(note)) = tagged.get("data") else {
            return Err(FromValueError::new("Expected embedded Note"));
        };
        let get = |key: &str| {
            note.get(key)
                .cloned()
                .ok_or_else(|| FromValueError::new(format!("Missing content field {key}")))
        };
        let result = Self::Note {
            format: String::from_value(get(ATTR_NOTE_FORMAT)?)?,
            body: String::from_value(get(ATTR_NOTE_CONTENT)?)?,
            created_at: DateTime::from_value(get(ATTR_CREATED_AT)?)?,
            updated_at: DateTime::from_value(get(ATTR_UPDATED_AT)?)?,
        };
        result
            .validate()
            .map_err(|e| FromValueError::new(e.message))?;
        Ok(result)
    }
}
