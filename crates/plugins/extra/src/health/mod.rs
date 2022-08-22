#[cfg(feature = "ui")]
mod ui;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use factdb::{
    macros::{Attribute, Class},
    Attribute, AttributeMeta, Class, ClassAttribute, ClassMeta, DbSchema, Expr, Id, IdOrIdent,
    Order, Select, Timestamp, ValueType,
};
use semantic_core::{
    base::{AttrComment, AttrDateTime},
    plugin::{Plugin, PluginDescriptor},
};

#[derive(Attribute)]
#[factor(namespace = "semantic_health", title = "Weight")]
pub struct AttrWeight(f64);

#[derive(Serialize, Deserialize, Class, Clone, Debug)]
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
                classes: vec![WeightLogEntry::schema()],
                indexes: vec![],
            }),
            import_matchers: vec![],
        }
    }

    fn migrations(
        &self,
        _already_applied_migrations: &HashSet<String>,
    ) -> Vec<factdb::query::migrate::Migration> {
        vec![
            factdb::query::migrate::Migration::with_name("create_weight_schema".to_string())
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: AttrWeight::QUALIFIED_NAME.to_string(),
                    title: Some("Weight".to_string()),
                    description: None,
                    value_type: ValueType::Float,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .entity_create(Class {
                    id: Id::nil(),
                    ident: WeightLogEntry::QUALIFIED_NAME.to_string(),
                    title: Some("Weight Log Entry".to_string()),
                    description: None,
                    attributes: vec![
                        ClassAttribute {
                            attribute: AttrWeight::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                        ClassAttribute {
                            attribute: AttrComment::QUALIFIED_NAME.to_string(),
                            required: false,
                        },
                        ClassAttribute {
                            attribute: AttrDateTime::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                }),
        ]
    }
}
