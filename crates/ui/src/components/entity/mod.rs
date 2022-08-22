pub mod browse_page;

use brass::dom::{builder::div, Tag, TagBuilder};
use factdb::DataMap;
use semantic_ui_core::{
    components::util::{buttons, notification_error, notification_warning, title_2, ButtonBuilder},
    context,
    routing::Route,
    EntityRenderOpts,
};

pub fn entity_create(entity_type: &str) -> TagBuilder {
    let renderer = context::registry()
        .entity_create_page_renderer(entity_type)
        .cloned();

    if let Some(renderer) = renderer {
        renderer(
            &DataMap::default(),
            &EntityRenderOpts {
                editable: true,
                preview: false,
            },
        )
    } else {
        notification_error().and(format!("Entity type {} not found.", entity_type))
    }
}

pub fn entity_create_page(entity_type: &str) -> TagBuilder {
    context::registry()
        .entities()
        .get(entity_type)
        .map(|info| {
            div()
                .and(title_2().and(format!("New {}", info.schema.pretty_name())))
                .and(entity_create(entity_type))
        })
        .unwrap_or_else(|| {
            div()
                .and(title_2().and("Create"))
                .and(notification_error().and(format!("Entity type '{}' not found.", entity_type)))
        })
}

pub fn create_page() -> TagBuilder {
    let reg = context::registry();

    let creatable = reg.creatable_entities();
    let entity_buttons = if creatable.is_empty() {
        notification_warning().and("No creatable entity types found.")
    } else {
        let entity_buttons = creatable.iter().map(|info| {
            let title = info.schema.pretty_name();
            let entity_type = info.schema.ident.clone();

            ButtonBuilder::new()
                .size_medium()
                .label(title)
                .on(move || {
                    context::router().goto(Route::EntityCreate {
                        entity_type: entity_type.clone(),
                    });
                })
                .build()
        });

        buttons().and_iter(entity_buttons)
    };

    div()
        .and(title_2().and("Create"))
        .and(
            buttons()
                .and(
                    ButtonBuilder::new()
                        .size_medium()
                        .icon("fas fa-globe")
                        .label("Import")
                        .on(|| {
                            context::router().goto(Route::Import { url: None });
                        })
                        .build(),
                )
                .and(
                    ButtonBuilder::new()
                        .size_medium()
                        .icon("fas fa-upload")
                        .label("Upload")
                        .on(|| {
                            context::router().goto(Route::Upload);
                        })
                        .build(),
                ),
        )
        .and(Tag::Hr.new())
        .and(entity_buttons)
}
