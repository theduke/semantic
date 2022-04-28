pub mod blob_cleanup;

use brass::{
    dom::{builder::div, TagBuilder},
    view,
};
use semantic_ui_core::{
    components::{
        loader::{load, Loader},
        util::{buttons, notification_success, title_2, ButtonBuilder, Cls},
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
            .and(div().and(link(Route::BlobCleanup, "Blob Manager").class(Cls::Button)))
            .and(media_analyze_toggle())
            .and(div().and(link(Route::BlobCleanup, "Blob Manager").class(Cls::Button)))
            .and(div().and(link(Route::Tags, "Tags").class(Cls::Button))),
    )
}

fn media_analyze_toggle() -> TagBuilder {
    let loader = Loader::<()>::new_idle();

    let loader2 = loader.clone();
    div()
        .and(
            buttons().and(
                ButtonBuilder::new()
                    .label("Start Media Analyzer")
                    .signal_loading(loader.signal_loading())
                    .on(move || {
                        loader2.spawn(async { context::api().analyze_media(false).await });
                    })
                    .build(),
            ),
        )
        .signal(loader.signal_render(|_| {
            notification_success()
                .and("Media analysis started in background.")
                .into_view()
        }))
        .bind(loader)
}

fn server_status() -> TagBuilder {
    let api = context::api();

    let f = async move { api.server_status().await };

    load(f, |status| {
        let backend = status.backend_status.as_ref().map(|s| {
            let db_size = s
                .db_size
                .map(|x| bytesize::ByteSize(x).to_string())
                .unwrap_or_else(|| "n/a".to_string());
            let asset_size = s
                .asset_size
                .map(|x| bytesize::ByteSize(x).to_string())
                .unwrap_or_else(|| "n/a".to_string());

            view! {
                div [
                    div [
                        "DB size: "
                        {db_size}
                    ]
                    div [
                        "Asset size: "
                        {asset_size}
                    ]
                ]
            }
        });

        view! {
            div(class="box") [
                div(class="subtitle is-4") [ "Status" ]
                {backend}
            ]
        }
    })
}
