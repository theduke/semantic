use factordb::{
    query::migrate::Migration,
    schema::{builtin::AttrIdent, AttributeDescriptor, EntityDescriptor},
    Attribute, Entity, Id,
};
use serde::{Deserialize, Serialize};

use crate::plugin::{Plugin, PluginDescriptor, PluginSchema};

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_code")]
pub struct AttrPluginCode(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_runtime")]
pub struct AttrPluginRuntime(String);

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct PluginSource {
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

pub struct SemanticCorePlugin;

impl PluginDescriptor for SemanticCorePlugin {
    const NAME: &'static str = "semantic/core";
    const IDENT: factordb::Ident = factordb::Ident::new_static(Self::NAME);

    fn new() -> crate::plugin::DynPlugin {
        std::sync::Arc::new(Self)
    }
}

impl Plugin for SemanticCorePlugin {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn migrations(&self) -> Vec<factordb::query::migrate::Migration> {
        let first = Migration::with_name("semantic/core/v1".to_string())
            .attr_create(AttrPluginCode::schema())
            .attr_create(AttrPluginRuntime::schema())
            .entity_create(PluginSource::schema());

        vec![first]
    }

    fn schema(&self) -> PluginSchema {
        PluginSchema {
            name: Self::NAME.into(),
            description: None,
            import_matchers: Vec::new(),
            db: Some(factordb::schema::DbSchema {
                attributes: vec![AttrPluginCode::schema(), AttrPluginRuntime::schema()],
                entities: vec![PluginSource::schema()],
                indexes: vec![],
            }),
        }
    }
}
