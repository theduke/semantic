use factordb::{data::DataMap, Attribute, Entity, Id};
use serde::{Deserialize, Serialize};

use super::AttrTitle;

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Note")]
pub struct AttrNoteBody(String);

#[derive(Serialize, Deserialize, Entity, Clone)]
#[factor(namespace = "semantic")]
pub struct Note {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,

    #[factor(attr = AttrNoteBody)]
    #[serde(rename = "semantic/note_body")]
    pub body: String,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
