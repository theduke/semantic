#[cfg(feature = "ui")]
mod ui;

use serde::{Deserialize, Serialize};

use factordb::{
    data::Timestamp,
    query::{
        expr::Expr,
        select::{Order, Select},
    },
    schema::{AttributeDescriptor, DbSchema, EntityDescriptor},
    Attribute, Entity, Id,
};
use semantic_core::{
    base::{AttrComment, AttrDateTime},
    plugin::{Plugin, PluginDescriptor},
};

#[derive(Attribute)]
#[factor(namespace = "semantic_health", title = "Weight")]
pub struct AttrWeight(f64);

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic_health")]
pub struct WeightLogEntry {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrWeight)]
    #[serde(rename = "semantic_health/weight")]
    pub weight: f64,

    #[factor(attr = AttrComment)]
    #[serde(rename = "semantic/comment")]
    pub comment: Option<String>,

    #[factor(attr = AttrDateTime)]
    #[serde(rename = "semantic/datetime")]
    pub datetime: Timestamp,
}

impl WeightLogEntry {
    pub fn query_all() -> Select {
        Select::new()
            .with_filter(Expr::is_entity::<WeightLogEntry>())
            .with_sort(AttrWeight::expr(), Order::Desc)
            .with_limit(356 * 20)
    }
}

pub struct HealthPlugin;

impl PluginDescriptor for HealthPlugin {
    const NAME: &'static str = "semantic_health";
    const IDENT: factordb::Ident = factordb::Ident::new_static(Self::NAME);

    fn new() -> semantic_core::plugin::DynPlugin {
        std::sync::Arc::new(Self)
    }
}

impl Plugin for HealthPlugin {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn schema(&self) -> semantic_core::plugin::PluginSchema {
        semantic_core::plugin::PluginSchema {
            name: self.name().into(),
            description: None,
            db: Some(DbSchema {
                attributes: vec![AttrWeight::schema()],
                entities: vec![WeightLogEntry::schema()],
                indexes: vec![],
            }),
            import_matchers: vec![],
        }
    }

    fn migrations(&self) -> Vec<factordb::query::migrate::Migration> {
        vec![
            factordb::query::migrate::Migration::with_name("create_weight_schema".to_string())
                .attr_create(AttrWeight::schema())
                .entity_create(WeightLogEntry::schema()),
        ]
    }
}
