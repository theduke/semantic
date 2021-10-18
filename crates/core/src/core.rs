use factordb::{schema::builtin::AttrIdent, Attribute, Entity, Id};
use serde::{Deserialize, Serialize};

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_code")]
pub struct AttrPluginCode(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_runtime")]
pub struct AttrPluginRuntime(String);

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Plugin {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "semantic/ident")]
    pub ident: String,

    #[factor(attr = AttrPluginRuntime)]
    #[serde(rename = "semantic/plugin_runtime")]
    pub runtime: Option<String>,

    #[factor(attr = AttrPluginCode)]
    #[serde(rename = "semantic/plugin_code")]
    pub code: Option<String>,
}
