use anyhow::bail;
use factdb::{
    macros::{Attribute, Class},
    AttributeMeta, DataMap, Id, Patch,
};
use serde::{Deserialize, Serialize};

use super::{AttrTitle, TextFormat};

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Note")]
pub struct AttrNoteBody(String);

#[derive(Serialize, Deserialize, Class, Clone)]
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

    #[factor(attr = TextFormat)]
    #[serde(rename = "semantic/text_format")]
    pub format: TextFormat,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

impl Note {
    pub fn build_patch(old: &Self, new: &Self) -> Result<Patch, anyhow::Error> {
        if old.id != new.id {
            bail!("Note ID mismatch");
        }
        let mut patch = Patch::new();
        if old.title != new.title {
            patch = patch.replace(AttrTitle::QUALIFIED_NAME, new.title.clone());
        }
        if old.body != new.body {
            patch = patch.replace(AttrNoteBody::QUALIFIED_NAME, new.body.clone());
        }

        Ok(patch)
    }
}
