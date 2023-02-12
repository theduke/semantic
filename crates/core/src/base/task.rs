use factdb::{
    macros::{Attribute, Class},
    DataMap, Id, Timestamp,
};
use serde::{Deserialize, Serialize};

use super::{AttrCreatedAt, AttrMarkdownBody, AttrSortOrder, AttrTitle, AttrUpdatedAt, TextFormat};

#[derive(Attribute)]
#[factor(
    namespace = "semantic.task",
    name = "completed_at",
    index,
    title = "Completed at"
)]
pub struct AttrTaskCompletedAt(Timestamp);

#[derive(Attribute)]
#[factor(
    namespace = "semantic.task",
    name = "due_date",
    index,
    title = "Due at"
)]
pub struct AttrTaskDueDate(Timestamp);

#[derive(Attribute)]
#[factor(
    namespace = "semantic.task",
    name = "priority",
    index,
    title = "Priority"
)]
pub struct AttrTaskPriority(i32);

#[derive(Serialize, Deserialize, Class, Clone)]
#[factor(namespace = "semantic")]
pub struct Task {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,

    #[factor(attr = AttrMarkdownBody)]
    #[serde(rename = "semantic/markdown_body")]
    pub body: Option<String>,

    #[factor(attr = TextFormat)]
    #[serde(rename = "semantic/text_format")]
    pub format: TextFormat,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Timestamp,

    #[factor(attr = AttrUpdatedAt)]
    #[serde(rename = "semantic/updated_at")]
    pub updated_at: Option<Timestamp>,

    #[factor(attr = AttrSortOrder)]
    #[serde(rename = "semantic/sort_order")]
    pub weight: Option<u64>,

    #[factor(attr = AttrUpdatedAt)]
    #[serde(rename = "semantic.task/completed_at")]
    pub completed_at: Option<Timestamp>,

    #[factor(attr = AttrTaskPriority)]
    #[serde(rename = "semantic.task/priority")]
    pub priority: Option<i64>,

    #[factor(attr = AttrTaskDueDate)]
    #[serde(rename = "semantic.task/due_date")]
    pub due_date: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
