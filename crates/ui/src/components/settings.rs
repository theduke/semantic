use brass::dom::{builder::div, TagBuilder};
use semantic_ui_core::{
    components::util::title_2,
    routing::{link, Route},
};

pub fn settings_page() -> TagBuilder {
    div()
        .and(title_2().and("Settings"))
        .and(link(Route::PluginManager, "Plugins"))
}
