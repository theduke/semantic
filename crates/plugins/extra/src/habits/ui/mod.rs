mod habit_create;
mod habit_form;
mod habit_view;
mod habits_dashboard;

use std::rc::Rc;

use brass::dom::{Render, TagBuilder};
use factordb::schema::EntityDescriptor;
use semantic_core::plugin::PluginDescriptor;
use semantic_ui_core::{
    plugin::PluginMainRoute,
    routing::{PluginRoute, PluginRouter},
    BrowserPlugin,
};

use super::{Habit, HabitOccurence};

impl BrowserPlugin for super::HabitsPlugin {
    fn spec(&self) -> semantic_ui_core::BrowserPluginSpec {
        semantic_ui_core::BrowserPluginSpec {
            name: Self::NAME.to_string(),
            main_route: Some(PluginMainRoute {
                name: "Habits".to_string(),
                route: HabitRouter::route_dashboard(),
            }),
        }
    }

    fn register(&self, registry: &mut semantic_ui_core::Registry) {
        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Create Habit".to_string(),
            entity_type: Habit::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::CreatePage,
            renderer: Rc::new(habit_create::habit_create_page),
            is_default: false,
        });

        registry.ignore_entity_type(HabitOccurence::QUALIFIED_NAME.into());
    }

    fn router(&self) -> Option<semantic_ui_core::routing::DynPluginRouter> {
        Some(Box::new(HabitRouter))
    }
}

struct HabitRouter;

impl HabitRouter {
    fn route_dashboard() -> PluginRoute {
        PluginRoute {
            path: "/habits".to_string(),
            title: "Habits".to_string(),
            render: std::rc::Rc::new(|| {
                TagBuilder::from_node(
                    habits_dashboard::HabitDashboard {}
                        .render()
                        .into_node()
                        .unwrap(),
                )
            }),
        }
    }
}

impl PluginRouter for HabitRouter {
    fn parse_path(&self, path: &[&str]) -> Option<semantic_ui_core::routing::PluginRoute> {
        match path {
            ["habits"] => Some(Self::route_dashboard()),
            _ => None,
        }
    }
}
