mod db;
use factordb::{
    prelude::{AttributeSchema, EntityAttribute, EntitySchema, Id, IdOrIdent, ValueType},
    schema::{AttributeDescriptor, DbSchema, EntityDescriptor},
};
use semantic_core::{
    base::{AttrDescription, AttrTitle},
    plugin::{Plugin, PluginDescriptor},
};

pub use self::db::*;

#[cfg(feature = "ui")]
mod ui;

pub struct HabitsPlugin;

impl PluginDescriptor for HabitsPlugin {
    const NAME: &'static str = "semantic_habits";
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::NAME);

    fn new() -> semantic_core::plugin::DynPlugin {
        std::sync::Arc::new(Self)
    }
}

impl Plugin for HabitsPlugin {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn schema(&self) -> semantic_core::plugin::PluginSchema {
        semantic_core::plugin::PluginSchema {
            name: self.name().into(),
            description: None,
            db: Some(DbSchema {
                attributes: vec![
                    AttrHabitOccurenceComment::schema(),
                    AttrHabitOccurenceParentId::schema(),
                    AttrHabitOccurenceTime::schema(),
                    HabitMode::schema(),
                ],
                entities: vec![Habit::schema(), HabitOccurence::schema()],
                indexes: vec![],
            }),
            import_matchers: vec![],
        }
    }

    fn migrations(&self) -> Vec<factordb::query::migrate::Migration> {
        vec![
            factordb::query::migrate::Migration::with_name("habits_create".to_string())
                .attr_create(AttributeSchema {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceComment::QUALIFIED_NAME.to_string(),
                    title: Some("Comment".to_string()),
                    description: None,
                    value_type: ValueType::String,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(AttributeSchema {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceParentId::QUALIFIED_NAME.to_string(),
                    title: Some("Duration".to_string()),
                    description: None,
                    value_type: ValueType::Ref,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(AttributeSchema {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceTime::QUALIFIED_NAME.to_string(),
                    title: Some("Time".to_string()),
                    description: None,
                    value_type: ValueType::DateTime,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(factordb::schema::AttributeSchema {
                    id: Id::nil(),
                    ident: HabitMode::QUALIFIED_NAME.to_string(),
                    title: Some("Habit Mode".into()),
                    description: None,
                    value_type: ValueType::Union(vec![
                        ValueType::Const("positive".into()),
                        ValueType::Const("negative".into()),
                        ValueType::Const("neutral".into()),
                    ]),
                    unique: false,
                    index: false,
                    strict: true,
                })
                .entity_create(EntitySchema {
                    id: Id::nil(),
                    ident: Habit::QUALIFIED_NAME.to_string(),
                    title: Some("Habit".to_string()),
                    description: None,
                    attributes: vec![
                        EntityAttribute {
                            attribute: AttrTitle::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                        EntityAttribute {
                            attribute: AttrDescription::IDENT,
                            cardinality: factordb::prelude::Cardinality::Optional,
                        },
                        EntityAttribute {
                            attribute: HabitMode::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                })
                .entity_create(EntitySchema {
                    id: Id::nil(),
                    ident: HabitOccurence::QUALIFIED_NAME.to_string(),
                    title: Some("Habit Occurence".to_string()),
                    description: None,
                    attributes: vec![
                        EntityAttribute {
                            attribute: AttrHabitOccurenceParentId::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                        EntityAttribute {
                            attribute: AttrHabitOccurenceTime::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                        EntityAttribute {
                            attribute: HabitMode::IDENT,
                            cardinality: factordb::prelude::Cardinality::Required,
                        },
                        EntityAttribute {
                            attribute: AttrHabitOccurenceComment::IDENT,
                            cardinality: factordb::prelude::Cardinality::Optional,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                }),
        ]
    }
}
