use brass::{dom::{Render, TagBuilder, builder::div}, signal::signal::SignalExt};
use semantic_ui_core::{
    components::{
        entity::entity_page,
        util::{buttons, container, Cls},
    },
    routing::{link, Route},
};

use brass::component::Component;

pub fn root() -> TagBuilder {
    let s = semantic_ui_core::context::router()
        .signal()
        .map(render_route);
    div().and(navbar()).child_signal(s)
}

fn render_route(route: Route) -> TagBuilder {
    tracing::trace!(?route, "showing route");

    let content = match route {
        // Special initialization routes
        Route::Browse => crate::components::entity::browse_page::BrowsePage::build(
            crate::components::entity::browse_page::BrowsePageProps {},
        ),
        Route::Import => super::import::import_page::ImportPage{}.render(),
        Route::Upload => super::upload::upload_page(),
        Route::Logout => todo!(),
        Route::Entity(ident) => {
            // FIXME: handle ident!
            tracing::trace!(?ident, "ident");
            let id = ident.as_id().unwrap();
            entity_page(id)
        }
        Route::EntityCreateSelect => todo!(),
        Route::EntityCreate { entity_type: _ } => todo!(),
        Route::Play => todo!(),
        Route::Tags => {
            semantic_ui_core::base::tags::tag_manager()
        }
    };

    container().and(content)
}

fn navbar() -> TagBuilder {
    let brand = div()
        .class("navbar-brand")
        .and(link(Route::Browse, "Semantic").class("navbar-item"));

    let items = div()
        .class("navbar-start")
        .and(link(Route::Browse, "Browse").class("navbar-item"))
        .and(link(Route::EntityCreateSelect, "Create").class("navbar-item"))
        .and(link(Route::Upload, "Upload").class("navbar-item"))
        .and(link(Route::Import, "Import").class("navbar-item"))
        .and(link(Route::Tags, "Tags").class("navbar-item"))
        .and(link(Route::Play, "Play").class("navbar-item"));

    let logout = link(Route::Logout, "Logout").class("navbar-item");

    let actions = buttons().and(logout);
    let end = div()
        .class("navbar-end")
        .and(div().class("navbar-item").and(actions));

    let menu = div()
        .class("navbar-menu")
        .class(Cls::IsActive)
        .and((items, end));

    div().class("navbar").and((brand, menu))
}
