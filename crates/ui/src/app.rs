use std::cell::RefCell;

use dioxus::prelude::*;
use semantic_rpc::RpcClient;
use semantic_ui_core::{UiCatalogProvider, provide_rpc_client, provide_ui_scope_context};

use crate::screens::{CatalogScreen, CollectionScreen, EntityScreen, HomeScreen, QueryScreen};

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
    Collection(String),
    Entity { collection: String, id: String },
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
                    button { onclick: move |_| page.set(Page::Home), "Home" }
                    button { onclick: move |_| page.set(Page::Catalog), "Catalog" }
                    button { onclick: move |_| page.set(Page::Collection("entities".to_string())), "Entities" }
                    button { onclick: move |_| page.set(Page::Query), "Query" }
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
                    Page::Collection(collection) => rsx! {
                        CollectionScreen {
                            collection: collection.clone(),
                            on_open_entity: move |(collection, id): (String, String)| page.set(Page::Entity { collection, id })
                        }
                    },
                    Page::Entity { collection, id } => rsx! {
                        EntityScreen {
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
