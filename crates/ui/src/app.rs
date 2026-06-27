use std::cell::RefCell;

use dioxus::prelude::*;
use semantic_rpc::RpcClient;
use semantic_ui_core::{UiCatalogProvider, provide_rpc_client, provide_ui_scope_context};

const CORE_STYLES: Asset = asset!("/assets/core_styles.css");

use crate::views::Route;

thread_local! {
    static BOOT: RefCell<Option<AppRootProps>> = const { RefCell::new(None) };
}

#[derive(Clone, Props)]
pub struct AppRootProps {
    pub client: RpcClient,
    pub initial_scope_id: Option<String>,
}

impl PartialEq for AppRootProps {
    fn eq(&self, other: &Self) -> bool {
        self.initial_scope_id == other.initial_scope_id
    }
}

pub fn launch_with_client(client: RpcClient, initial_scope_id: Option<String>) {
    BOOT.with(|boot| {
        *boot.borrow_mut() = Some(AppRootProps {
            client,
            initial_scope_id,
        });
    });
    dioxus::launch(boot_app);
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
            initial_scope_id: props.initial_scope_id
        }
    }
}

#[component]
pub fn AppRoot(props: AppRootProps) -> Element {
    provide_rpc_client(props.client);
    provide_ui_scope_context(props.initial_scope_id);
    rsx! {
        document::Stylesheet { href: CORE_STYLES }
        dxcomp::Stylesheet {}
        UiCatalogProvider {
            Router::<Route> {}
        }
    }
}
