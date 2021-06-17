use factordb::{Attribute, Entity, Id};
use serde::{Deserialize, Serialize};

use super::AttrTitle;

#[derive(Attribute)]
#[factor(namespace = "semantic")]
pub struct AttrNoteBody(String);

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic")]
pub struct Note {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantics/title")]
    pub title: String,

    #[factor(attr = AttrNoteBody)]
    #[serde(rename = "semantics/note_body")]
    pub body: String,
}
