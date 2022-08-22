mod db;
use std::collections::HashSet;

use factdb::{
    Attribute, AttributeMeta, Class, ClassAttribute, ClassMeta, DbSchema, Id, IdOrIdent, ValueType,
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
                classes: vec![Habit::schema(), HabitOccurence::schema()],
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
            factdb::query::migrate::Migration::with_name("habits_create".to_string())
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceComment::QUALIFIED_NAME.to_string(),
                    title: Some("Comment".to_string()),
                    description: None,
                    value_type: ValueType::String,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceParentId::QUALIFIED_NAME.to_string(),
                    title: Some("Duration".to_string()),
                    description: None,
                    value_type: ValueType::Ref,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: AttrHabitOccurenceTime::QUALIFIED_NAME.to_string(),
                    title: Some("Time".to_string()),
                    description: None,
                    value_type: ValueType::DateTime,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(factdb::schema::Attribute {
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
                .entity_create(Class {
                    id: Id::nil(),
                    ident: Habit::QUALIFIED_NAME.to_string(),
                    title: Some("Habit".to_string()),
                    description: None,
                    attributes: vec![
                        ClassAttribute {
                            attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                        ClassAttribute {
                            attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                            required: false,
                        },
                        ClassAttribute {
                            attribute: HabitMode::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                })
                .entity_create(Class {
                    id: Id::nil(),
                    ident: HabitOccurence::QUALIFIED_NAME.to_string(),
                    title: Some("Habit Occurence".to_string()),
                    description: None,
                    attributes: vec![
                        ClassAttribute {
                            attribute: AttrHabitOccurenceParentId::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                        ClassAttribute {
                            attribute: AttrHabitOccurenceTime::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                        ClassAttribute {
                            attribute: HabitMode::QUALIFIED_NAME.to_string(),
                            required: true,
                        },
                        ClassAttribute {
                            attribute: AttrHabitOccurenceComment::QUALIFIED_NAME.to_string(),
                            required: false,
                        },
                    ],
                    extends: vec![],
                    strict: false,
                }),
        ]
    }
}
