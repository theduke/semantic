use brass::dom::{builder::div, TagBuilder};
use semantic_ui_core::{
    components::{
        loader::load,
        util::{box_, subtitle_4, title_2, Cls},
    },
    context,
    routing::{link, Route},
};

pub fn settings_page() -> TagBuilder {
    div().and(title_2().and("Settings")).and(
        div()
            .style_raw("display: flex; flex-direction: column; gap: 1rem;")
            .and(server_status())
            .and(div().and(link(Route::PluginManager, "Plugins").class(Cls::Button)))
            .and(div().and(link(Route::Tags, "Tags").class(Cls::Button))),
    )
}

fn server_status() -> TagBuilder {
    let api = context::api();

    let f = async move { api.server_status().await };

    load(f, |status| {
        let backend = status.backend_status.as_ref().map(|s| {
            div().and(
                div()
                    .and(div().and(format!(
                        "DB size: {}",
                        s.db_size
                            .map(|x| bytesize::ByteSize(x).to_string())
                            .unwrap_or_else(|| "n/a".to_string())
                    )))
                    .and(div().and(format!(
                        "Asset size: {}",
                        s.asset_size
                            .map(|x| bytesize::ByteSize(x).to_string())
                            .unwrap_or_else(|| "n/a".to_string())
                    )))
                    .and(div().and(format!(
                        "Storage size: {}",
                        s.storage_size
                            .map(|x| bytesize::ByteSize(x).to_string())
                            .unwrap_or_else(|| "n/a".to_string())
                    ))),
            )
        });

        box_()
            .and(subtitle_4().and("Status"))
            .and(backend)
            .into_view()
    })
}
