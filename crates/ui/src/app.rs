use std::cell::RefCell;

use dioxus::prelude::*;
use semantic_rpc::RpcClient;
use semantic_ui_core::{UiCatalogProvider, provide_rpc_client, provide_ui_scope_context};

const CORE_STYLES: Asset = asset!("/assets/core_styles.css");

use crate::screens::{
    CatalogScreen, CollectionScreen, CreateEntityScreen, EditEntityScreen, EntityScreen,
    HomeScreen, QueryScreen,
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Page {
    Home,
    Catalog,
    CreateEntity,
    Collection(String),
    Entity { collection: String, id: String },
    EditEntity { collection: String, id: String },
    Query,
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
    let client = semantic_rpc::transport::http_client::HttpRpcClient::new("/rpc").into();
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
            AppShell {}
        }
    }
}

#[component]
fn AppShell() -> Element {
    let mut page = use_signal(|| Page::Home);
    let current_page = page.read().clone();
    rsx! {
        div { class: "semantic-ui",
            header { class: "semantic-ui__header",
                h1 { "Semantic" }
                nav {
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: move |_| page.set(Page::Home), "Home" }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: move |_| page.set(Page::Catalog), "Catalog" }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: move |_| page.set(Page::Collection("entities".to_string())), "Entities" }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: move |_| page.set(Page::CreateEntity), "Create" }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: move |_| page.set(Page::Query), "Query" }
                }
            }
            main { class: "semantic-ui__main",
                match current_page {
                    Page::Home => rsx! {
                        HomeScreen {
                            on_open_collection: move |collection: String| page.set(Page::Collection(collection))
                        }
                    },
                    Page::Catalog => rsx! { CatalogScreen {} },
                    Page::CreateEntity => rsx! {
                        CreateEntityScreen {
                            on_open_entity: move |(collection, id): (String, String)| {
                                page.set(Page::Entity { collection, id })
                            }
                        }
                    },
                    Page::Collection(collection) => rsx! {
                        CollectionScreen {
                            collection: collection.clone(),
                            on_open_entity: move |(collection, id): (String, String)| {
                                page.set(Page::Entity { collection, id })
                            },
                            on_edit_entity: move |(collection, id): (String, String)| {
                                page.set(Page::EditEntity { collection, id })
                            },
                            on_create_entity: move |()| page.set(Page::CreateEntity)
                        }
                    },
                    Page::Entity { collection, id } => rsx! {
                        EntityScreen {
                            collection,
                            id,
                            on_edit_entity: move |(collection, id): (String, String)| {
                                page.set(Page::EditEntity { collection, id })
                            }
                        }
                    },
                    Page::EditEntity { collection, id } => rsx! {
                        EditEntityScreen {
                            collection,
                            id
                        }
                    },
                    Page::Query => rsx! { QueryScreen {} },
                }
            }
        }
    }
}
