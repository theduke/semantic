mod db;
use factordb::{
    prelude::IdOrIdent,
    schema::{AttributeDescriptor, DbSchema, EntityDescriptor},
};
use semantic_core::plugin::{Plugin, PluginDescriptor};

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
                .attr_create(AttrHabitOccurenceComment::schema())
                .attr_create(AttrHabitOccurenceParentId::schema())
                .attr_create(AttrHabitOccurenceTime::schema())
                .attr_create(HabitMode::schema())
                .entity_create(Habit::schema())
                .entity_create(HabitOccurence::schema()),
        ]
    }
}
