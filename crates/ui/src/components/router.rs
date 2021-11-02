use brass::{
    vdom::{self, component, div, s, Render, TagBuilder},
    Str, VNode,
};
use semantic_ui_core::{routing::Route, ContextExt};

use crate::components as comps;

use super::{base::tags::TagManager, import::import_page::ImportPage};

pub fn router(route: &Route) -> VNode {
    let nav = navbar();
    let content = match route {
        Route::Browse => component::<comps::entity::browse_page::BrowsePage>(
            comps::entity::browse_page::BrowsePageProps {},
        ),
        Route::Logout => VNode::Empty,
        Route::Import => component::<ImportPage>(()),
        Route::Upload => comps::upload::upload_page(),
        Route::Entity(ident) => component::<comps::entity::entity_page::EntityPage>(
            comps::entity::entity_page::EntityPageProps {
                ident: ident.clone(),
            },
        ),
        Route::EntityCreateSelect => {
            component::<comps::entity::entity_create_selector::EntityCreateSelectorPage>(())
        }
        Route::EntityCreate { entity_type } => {
            component::<comps::entity::entity_create_page::EntityCreatePage>(
                comps::entity::entity_create_page::EntityCreatePageProps {
                    entity_type: entity_type.clone(),
                },
            )
        }
        Route::Tags => TagManager {}.render(),
        Route::Play => {
            // Special casing for play because of overflow: hidden;
            // TODO: the router should probably just use the raw output, without
            // wrapping in in a .container below...
            let content = comps::base::play::StandalonePlayer {
                expr: None,
                keyboard_controls: true,
            }
            .render();

            return div()
                .style_raw("height: 100%; display: flex; flex-direction: column;")
                .and(nav)
                .and(
                    div()
                        .class("container")
                        .style_raw("width: 100%; flex-grow: 1; overflow: hidden;")
                        .and(content),
                )
                .build();
        }
    };

    div()
        .style_raw("height: 100%; display: flex; flex-direction: column;")
        .and(nav)
        .and(
            div()
                .class("container")
                .style_raw("width: 100%; flex-grow: 1;")
                .and(content),
        )
        .build()
}
