use std::cell::RefCell;

use dioxus::prelude::*;
use semantic_rpc::RpcClient;
use semantic_ui_core::{
    RenderSettings, UiCatalogProvider, provide_rpc_client, provide_ui_scope_context,
};

const CORE_STYLES: Asset = asset!("/assets/core_styles.css");

use crate::views::Route;

thread_local! {
    static BOOT: RefCell<Option<AppRootProps>> = const { RefCell::new(None) };
}

#[derive(Clone, Props)]
pub struct AppRootProps {
    pub client: RpcClient,
    pub initial_scope_id: Option<String>,
    pub file_api_prefix: Option<String>,
}

impl PartialEq for AppRootProps {
    fn eq(&self, other: &Self) -> bool {
        self.initial_scope_id == other.initial_scope_id
            && self.file_api_prefix == other.file_api_prefix
    }
}

pub fn launch_with_client(client: RpcClient, initial_scope_id: Option<String>) {
    let file_api_prefix = file_api_prefix_from_probe(&client);
    launch_with_client_and_file_api(client, initial_scope_id, file_api_prefix);
}

pub fn launch_with_client_and_file_api(
    client: RpcClient,
    initial_scope_id: Option<String>,
    file_api_prefix: Option<String>,
) {
    BOOT.with(|boot| {
        *boot.borrow_mut() = Some(AppRootProps {
            client,
            initial_scope_id,
            file_api_prefix,
        });
    });
    dioxus::launch(boot_app);
}

#[cfg(feature = "desktop")]
pub fn launch_with_client_file_api_and_config(
    client: RpcClient,
    initial_scope_id: Option<String>,
    file_api_prefix: Option<String>,
    config: dioxus::desktop::Config,
) {
    BOOT.with(|boot| {
        *boot.borrow_mut() = Some(AppRootProps {
            client,
            initial_scope_id,
            file_api_prefix,
        });
    });
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(boot_app);
}

#[cfg(target_arch = "wasm32")]
pub fn launch_web() {
    let client = semantic_rpc::transport::http_client::HttpRpcClient::new("/api/v1/rpc").into();
    launch_with_client(client, Some("default".to_string()));
}

fn boot_app() -> Element {
    let props = BOOT
        .with(|boot| boot.borrow_mut().take())
        .expect("semantic UI launch state missing");
    rsx! {
        AppRoot {
            client: props.client,
            initial_scope_id: props.initial_scope_id,
            file_api_prefix: props.file_api_prefix
        }
    }
}

#[component]
pub fn AppRoot(props: AppRootProps) -> Element {
    provide_rpc_client(props.client);
    provide_ui_scope_context(props.initial_scope_id);
    let render_settings = props.file_api_prefix.map(|file_api_prefix| {
        let mut settings = RenderSettings::default();
        settings.file_api_prefix = file_api_prefix;
        settings
    });
    rsx! {
        document::Stylesheet { href: CORE_STYLES }
        dxcomp::Stylesheet {}
        UiCatalogProvider { render_settings,
            Router::<Route> {}
        }
    }
}

fn file_api_prefix_from_probe(client: &RpcClient) -> Option<String> {
    let probe = "__semantic_probe__";
    let url = client.file_url(probe)?;
    url.strip_suffix(probe)
        .map(|prefix| prefix.trim_end_matches('/').to_string())
}
