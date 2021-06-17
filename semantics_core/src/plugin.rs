use factordb::{schema::builtin::AttrIdent, Attribute, Entity, Id, Ident};
use serde::{Deserialize, Serialize};

use super::{AttrPreviewImageUrl, AttrTitle, AttrUrl};

#[derive(Attribute)]
#[factor(namespace = "semantics")]
pub struct AttrBlobUri(String);

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantics")]
pub struct Plugin {
    #[factor(attr = AttrId)]
    pub id: Id,
    #[factor(attr = AttrIdent)]
    pub ident: Ident,
}
