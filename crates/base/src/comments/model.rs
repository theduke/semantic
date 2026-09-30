use super::schema::*;
use crate::{
    content::{ATTR_MAIN_CONTENT, MainContent},
    domain_support::{optional, read},
};
use semantic_data::{
    attr::{ATTR_CREATED_AT, ATTR_PARENT, ATTR_UPDATED_AT},
    value::{DateTime, FromValue, IntoValue, Object, SemanticType, Value},
};
use semantic_rpc_core::RpcError;
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct EntityTarget {
    pub collection: String,
    pub id: String,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub main_content: MainContent,
    pub author_id: String,
    pub parent: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub deleted: bool,
}
impl Comment {
    pub fn from_object(o: &Object) -> Result<Self, RpcError> {
        if o.get("type").and_then(Value::as_str) != Some(CLASS_ID) {
            return Err(crate::domain_support::error("Entity is not a comment"));
        }
        Ok(Self {
            id: read(o, "id")?,
            main_content: read(o, ATTR_MAIN_CONTENT)?,
            author_id: read(o, ATTR_AUTHOR_ID)?,
            parent: optional(o, ATTR_PARENT)?,
            created_at: read(o, ATTR_CREATED_AT)?,
            updated_at: read(o, ATTR_UPDATED_AT)?,
            deleted: read(o, ATTR_DELETED)?,
        })
    }
    pub fn to_object(&self) -> Object {
        let mut o = Object::new();
        for (key, value) in [
            ("id", self.id.clone().into_value()),
            ("type", CLASS_ID.to_owned().into_value()),
            (ATTR_MAIN_CONTENT, self.main_content.clone().into_value()),
            (ATTR_AUTHOR_ID, self.author_id.clone().into_value()),
            (ATTR_CREATED_AT, self.created_at.into_value()),
            (ATTR_UPDATED_AT, self.updated_at.into_value()),
            (ATTR_DELETED, self.deleted.into_value()),
        ] {
            o.insert(key, value);
        }
        if let Some(parent) = &self.parent {
            o.insert(ATTR_PARENT, parent.clone().into_value());
        }
        o
    }
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub total: u64,
}
