use factdb::{macros::Class, AttrIdent, Expr, Id, Select};

use super::{AttrParent, AttrTitle};

#[derive(serde::Serialize, serde::Deserialize, Class, Clone)]
#[factor(namespace = "semantic")]
pub struct Container {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,
}

impl Container {
    pub fn select_children(&self) -> Select {
        Select::new().with_filter(Expr::eq(Expr::attr::<AttrParent>(), self.id))
    }
}
