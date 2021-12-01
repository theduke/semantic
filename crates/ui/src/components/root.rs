use brass::{
    dom::{builder::div, Render, TagBuilder, View},
    signal::signal::SignalExt,
};
use semantic_ui_core::{
    components::{
        entity::entity_page,
        util::{buttons, container, icon_fa},
    },
    routing::{link, Route},
};

use brass::component::Component;

use crate::components::player::StandalonePlayer;

pub fn root() -> TagBuilder {
    let s = semantic_ui_core::context::router()
        .signal()
        .map(render_route);
    div()
        .style_raw("display: flex; flex-direction: column; height: 100%; width: 100%;")
        .and(navbar())
        .child_signal(s)
}

fn render_route(route: Route) -> View {
    let content = match route {
        // Special initialization routes
        Route::Browse => crate::components::entity::browse_page::BrowsePage::build(
            crate::components::entity::browse_page::BrowsePageProps {},
        ),
        Route::Import { url } => super::import::import_page::ImportPage { url }.render(),
        Route::Upload => super::upload::upload_page().into_view(),
        Route::Logout => super::login::logout().into_view(),
        Route::Entity(ident) => {
            // FIXME: handle ident!
            tracing::trace!(?ident, "ident");
            let id = ident.as_id().unwrap();
            entity_page(id).into_view()
        }
        Route::Create => super::entity::create_page().into_view(),
        Route::EntityCreate { entity_type } => {
            super::entity::entity_create_page(&entity_type).into_view()
        }
        Route::Play => {
            // Do not show container for player.
            return StandalonePlayer {
                filter: None,
                keyboard_controls: true,
            }
            .render();
        }
        Route::Tags => semantic_ui_core::base::tags::tag_manager().into_view(),
        Route::Plugin(route) => (route.render)().into_view(),
        Route::PluginManager => super::plugins::plugin_manager().into_view(),
        Route::PluginCreate => super::plugins::plugin_source_create_page().into_view(),
        Route::PluginTest => super::plugins::plugin_test_page().into_view(),
        Route::Settings => super::settings::settings_page().into_view(),
        Route::PluginUpdate { id } => super::plugins::plugin_source_update_page(id).into_view(),
        Route::Apps => super::plugins::plugin_main_routes().into_view(),
    };

    container()
        .style_raw("min-width: 800px;")
        .and(content)
        .into_view()
}

fn navbar() -> TagBuilder {
    let brand = div()
        .class("navbar-brand")
        .and(link(Route::Browse, "Semantic").class("navbar-item"));

    let items = div()
        .class("navbar-start")
        .and(link(Route::Browse, "Browse").class("navbar-item"))
        .and(link(Route::Create, "Create").class("navbar-item"))
        .and(link(Route::Play, "Play").class("navbar-item"))
        .and(link(Route::Apps, "Apps").class("navbar-item"));

    let logout = link(Route::Logout, "Logout").class("navbar-item");
    let settings = link(Route::Settings, icon_fa("fa-cog")).class("navbar-item");

    let actions = buttons().and(settings).and(logout);
    let end = div()
        .class("navbar-end")
        .and(div().class("navbar-item").and(actions));

    let menu = div()
        .class("navbar-menu")
        // .class(Cls::IsActive)
        .and((items, end));

    div()
        .class("navbar")
        .style_raw("height: 52px; border-bottom: 2px solid black; border-radius: 10px; margin-bottom: 1rem;")
        .and((brand, menu))
}
