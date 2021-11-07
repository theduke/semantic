use brass::dom::{builder::div, TagBuilder};
use semantic_ui_core::{
    components::util::{title_2, Cls},
    routing::{link, Route},
};

pub fn settings_page() -> TagBuilder {
    div().and(title_2().and("Settings")).and(
        div()
            .style_raw("display: flex; flex-direction: column; gap: 1rem;")
            .and(div().and(link(Route::PluginManager, "Plugins").class(Cls::Button)))
            .and(div().and(link(Route::Tags, "Tags").class(Cls::Button))),
    )
}
