#[cfg(feature = "ui")]
mod ui;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use factordb::prelude::{
    Attribute, AttributeDescriptor, AttributeSchema, DbSchema, Entity, EntityAttribute,
    EntityDescriptor, EntitySchema, Expr, Id, IdOrIdent, Order, Select, Timestamp, ValueType,
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
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::NAME);

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

    fn migrations(
        &self,
        _already_applied_migrations: &HashSet<String>,
    ) -> Vec<factordb::query::migrate::Migration> {
        vec![
            factordb::query::migrate::Migration::with_name("create_weight_schema".to_string())
                .attr_create(AttributeSchema {
                    id: Id::nil(),
                    ident: AttrWeight::QUALIFIED_NAME.to_string(),
                    title: Some("Weight".to_string()),
                    description: None,
                    value_type: ValueType::Float,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .entity_create(EntitySchema {
                    id: Id::nil(),
                    ident: WeightLogEntry::QUALIFIED_NAME.to_string(),
                    title: Some("Weight Log Entry".to_string()),
                    description: None,
                    attributes: vec![
                        EntityAttribute {
                            attribute: AttrWeight::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                        EntityAttribute {
                            attribute: AttrComment::IDENT,
                            cardinality: factordb::prelude::Cardinality::Optional,
                        },
                        EntityAttribute {
                            attribute: AttrDateTime::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                }),
        ]
    }
}
