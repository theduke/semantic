use std::rc::Rc;

use dioxus::prelude::{Element, EventHandler};
use semantic_data::schema::ClassType;
use semantic_data::value::Object;

use crate::ui_catalog::{EntityTarget, UiCatalog};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityActionPlacement {
    Card,
    Detail,
    BrowseRow,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dioxus::prelude::*;
    use semantic_data::schema::{ClassRef, ClassType, Meta};
    use semantic_data::value::Value;
    use semantic_db_core::catalog::{CatalogStorageSnapshot, LocalClassId, StoredClass};

    use super::*;
    use crate::ui_catalog::UiCatalogConfig;

    #[test]
    fn entity_action_filtering_handles_global_exact_inherited_unrelated_and_disabled() {
        let mut catalog = UiCatalog::builder(snapshot_with_classes(vec![
            class("base", None),
            class("child", Some("base")),
            class("other", None),
        ]))
        .with_config(UiCatalogConfig {
            register_default_renderers: false,
            register_default_form_renderers: false,
        })
        .build();
        catalog.register_entity_action(action("global", None, true));
        catalog.register_entity_action(action("exact", Some("child"), true));
        catalog.register_entity_action(action("inherited", Some("base"), true));
        catalog.register_entity_action(action("unrelated", Some("other"), true));
        catalog.register_entity_action(action("disabled", None, false));

        let mut object = Object::new();
        object.insert("type", Value::String("child".to_string()));
        let ctx = EntityActionContext {
            target: EntityTarget::default_collection("id-1"),
            object,
            class: catalog.class_by_id("child").cloned(),
            placement: EntityActionPlacement::Card,
            on_delete: None,
        };
        let ids = catalog
            .entity_actions_for(&ctx)
            .into_iter()
            .map(|action| action.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["global", "exact", "inherited"]);
    }

    fn action(id: &str, class_id: Option<&str>, enabled: bool) -> EntityActionRegistration {
        EntityActionRegistration {
            id: id.to_string(),
            label: id.to_string(),
            icon: None,
            class_id: class_id.map(str::to_string),
            placements: vec![EntityActionPlacement::Card],
            enabled: Rc::new(move |_| enabled),
            render: Rc::new(|_| rsx! { span {} }),
        }
    }

    fn class(id: &str, inherits: Option<&str>) -> ClassType {
        ClassType {
            id: id.to_string(),
            name: id.to_string(),
            inherits: inherits.map(|id| ClassRef { id: id.to_string() }),
            extends: Vec::new(),
            strict_schema: false,
            attributes: BTreeMap::new(),
            constraints: Vec::new(),
            meta: Meta::default(),
        }
    }

    fn snapshot_with_classes(classes: Vec<ClassType>) -> CatalogStorageSnapshot {
        CatalogStorageSnapshot {
            attributes: Vec::new(),
            type_defs: Vec::new(),
            record_types: Vec::new(),
            classes: classes
                .into_iter()
                .enumerate()
                .map(|(index, class)| StoredClass {
                    lid: LocalClassId::from(index),
                    class,
                })
                .collect(),
            collections: Vec::new(),
            indexes: Vec::new(),
            relationships: Vec::new(),
            packages: Vec::new(),
            applied_migrations: Vec::new(),
            next_field_id: 0,
            auto_index_enabled: false,
        }
    }
}

#[derive(Clone)]
pub struct EntityActionContext {
    pub target: EntityTarget,
    pub object: Object,
    pub class: Option<ClassType>,
    pub placement: EntityActionPlacement,
    pub on_delete: Option<EventHandler<EntityTarget>>,
}

#[derive(Clone)]
pub struct EntityActionRegistration {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub class_id: Option<String>,
    pub placements: Vec<EntityActionPlacement>,
    pub enabled: Rc<dyn Fn(&EntityActionContext) -> bool>,
    pub render: Rc<dyn Fn(EntityActionContext) -> Element>,
}

impl UiCatalog {
    pub fn register_entity_action(&mut self, action: EntityActionRegistration) {
        self.entity_actions_mut().push(action);
    }

    pub fn entity_actions_for(&self, ctx: &EntityActionContext) -> Vec<EntityActionRegistration> {
        self.entity_actions()
            .iter()
            .filter(|action| action.id != "open" || self.entity_navigation().open.is_some())
            .filter(|action| action.placements.contains(&ctx.placement))
            .filter(|action| self.entity_action_matches_class(action, ctx.class.as_ref()))
            .filter(|action| (action.enabled)(ctx))
            .cloned()
            .collect()
    }

    fn entity_action_matches_class(
        &self,
        action: &EntityActionRegistration,
        class: Option<&ClassType>,
    ) -> bool {
        let Some(action_class_id) = action.class_id.as_deref() else {
            return true;
        };
        let Some(class) = class else {
            return false;
        };
        self.class_inherits(&class.id, action_class_id)
    }
}
