use factordb::{
    data::{Timestamp, ValueType},
    schema::AttributeDescriptor,
    Attribute, Entity, Id,
};
use serde::{Deserialize, Serialize};

use super::{AttrDescription, AttrTitle};

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "habit_ocurrence_parent_id",
    title = "Duration"
)]
pub struct AttrHabitOccurenceParentId(Id);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "habit_ocurrence_time", title = "Time")]
pub struct AttrHabitOccurenceTime(Timestamp);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "habit_ocurrence_comment",
    title = "Comment"
)]
pub struct AttrHabitOccurenceComment(String);

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
pub enum HabitMode {
    #[serde(rename = "positive")]
    Positive,
    #[serde(rename = "negative")]
    Negative,
    #[serde(rename = "neutral")]
    Neutral,
}

impl AttributeDescriptor for HabitMode {
    const NAMESPACE: &'static str = "semantic";
    const PLAIN_NAME: &'static str = "habit_mode";
    const QUALIFIED_NAME: &'static str = "semantic/habit_mode";

    type Type = HabitMode;

    fn schema() -> factordb::schema::AttributeSchema {
        factordb::schema::AttributeSchema {
            id: Id::nil(),
            ident: Self::QUALIFIED_NAME.to_string(),
            title: Some("Habit Mode".into()),
            description: None,
            value_type: ValueType::Union(vec![
                ValueType::Const("positive".into()),
                ValueType::Const("negative".into()),
                ValueType::Const("neutral".into()),
            ]),
            unique: false,
            index: false,
            strict: true,
        }
    }
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Habit {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = HabitMode)]
    #[serde(rename = "semantic/description")]
    pub mode: HabitMode,
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct HabitOccurence {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrHabitOccurenceParentId)]
    #[serde(rename = "factor/habit_ocurrence_parent_id")]
    pub parent_id: Id,

    #[factor(attr = AttrHabitOccurenceTime)]
    #[serde(rename = "factor/habit_ocurrence_time")]
    pub time: Timestamp,

    #[factor(attr = HabitMode)]
    #[serde(rename = "semantic/description")]
    pub mode: HabitMode,

    #[factor(attr = AttrHabitOccurenceComment)]
    #[serde(rename = "semantic/habit_ocurrence_comment")]
    pub comment: Option<String>,
}
