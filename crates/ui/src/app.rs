use std::{cell::RefCell, rc::Rc};

use dioxus::prelude::*;
use dioxus_icons::lucide::Pencil;
use semantic_data::value::Value;
use semantic_rpc::RpcClient;
use semantic_ui_core::{
    EntityActionContext, EntityActionPlacement, EntityActionRegistration, EntityTarget,
    RenderSettings, UiCatalog, UiCatalogProvider, ValueRenderContext, provide_rpc_client,
    provide_ui_scope_context,
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
        self.client == other.client
            && self.initial_scope_id == other.initial_scope_id
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
        .with(|boot| boot.borrow().clone())
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
        UiCatalogProvider {
            render_settings,
            configure_catalog: configure_ui_catalog,
            Router::<Route> {}
        }
    }
}

fn configure_ui_catalog(mut catalog: UiCatalog) -> UiCatalog {
    semantic_ui_core::register_default_playback_renderers(&mut catalog);
    catalog
        .render_registry_mut()
        .register_type_renderer("ref", Rc::new(render_ref_link));
    catalog.set_entity_href_builder(Rc::new(entity_href));
    catalog.set_entity_link_renderer(Rc::new(entity_link));
    catalog.set_entity_open_handler(Rc::new(entity_open));
    register_entity_actions(&mut catalog);
    catalog
}

fn entity_href(target: &EntityTarget) -> Option<String> {
    if target.id.is_empty() {
        return None;
    }
    if target.is_default_collection() {
        Some(format!("/entities/{}", target.id))
    } else {
        Some(format!(
            "/collections/{}/{}",
            target.collection_or_default(),
            target.id
        ))
    }
}

fn entity_link(target: EntityTarget, content: Element) -> Element {
    if target.is_default_collection() {
        rsx! {
            Link {
                to: Route::DefaultEntityPage { id: target.id },
                class: "semantic-ref semantic-ref--link",
                {content}
            }
        }
    } else {
        rsx! {
            Link {
                to: Route::CollectionEntityPage {
                    collection: target.collection_or_default().to_string(),
                    id: target.id
                },
                class: "semantic-ref semantic-ref--link",
                {content}
            }
        }
    }
}

fn entity_open(target: EntityTarget) {
    if target.is_default_collection() {
        navigator().push(Route::DefaultEntityPage { id: target.id });
    } else {
        navigator().push(Route::CollectionEntityPage {
            collection: target.collection_or_default().to_string(),
            id: target.id,
        });
    }
}

fn register_entity_actions(catalog: &mut UiCatalog) {
    catalog.register_entity_action(EntityActionRegistration {
        id: "edit".to_string(),
        label: "Edit".to_string(),
        icon: None,
        class_id: None,
        placements: vec![
            EntityActionPlacement::Card,
            EntityActionPlacement::Detail,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx: &EntityActionContext| !ctx.target.id.is_empty()),
        render: Rc::new(|ctx: EntityActionContext| entity_edit_link(ctx.target)),
    });
}

fn entity_edit_link(target: EntityTarget) -> Element {
    if target.is_default_collection() {
        rsx! {
            Link {
                to: Route::DefaultEditEntityPage { id: target.id },
                class: "dx-button semantic-entity-action",
                "data-style": "outline",
                "data-size": "icon-sm",
                title: "Edit",
                aria_label: "Edit entity",
                Pencil { size: "1rem" }
            }
        }
    } else {
        rsx! {
            Link {
                to: Route::CollectionEditEntityPage {
                    collection: target.collection_or_default().to_string(),
                    id: target.id
                },
                class: "dx-button semantic-entity-action",
                "data-style": "outline",
                "data-size": "icon-sm",
                title: "Edit",
                aria_label: "Edit entity",
                Pencil { size: "1rem" }
            }
        }
    }
}

fn render_ref_link(ctx: ValueRenderContext) -> Element {
    let Value::String(id) = ctx.value else {
        let text = format!("{:?}", ctx.value);
        return rsx! { span { class: "semantic-value semantic-value--scalar", "{text}" } };
    };
    if id.is_empty() {
        return rsx! { span { class: "semantic-value semantic-value--scalar" } };
    }
    rsx! {
        Link {
            to: Route::DefaultEntityPage { id: id.clone() },
            class: "semantic-ref semantic-ref--link",
            "{id}"
        }
    }
}

fn file_api_prefix_from_probe(client: &RpcClient) -> Option<String> {
    let probe = "__semantic_probe__";
    let url = client.file_url(probe)?;
    url.strip_suffix(probe)
        .map(|prefix| prefix.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use semantic_data::builtin::DEFAULT_COLLECTION;

    use super::*;

    #[test]
    fn entity_href_builder_uses_default_and_collection_routes() {
        assert_eq!(
            entity_href(&EntityTarget::default_collection("id-1")).as_deref(),
            Some("/entities/id-1")
        );
        assert_eq!(
            entity_href(&EntityTarget::new(
                Some(DEFAULT_COLLECTION.to_string()),
                "id-1"
            ))
            .as_deref(),
            Some("/entities/id-1")
        );
        assert_eq!(
            entity_href(&EntityTarget::new(Some("custom".to_string()), "id-1")).as_deref(),
            Some("/collections/custom/id-1")
        );
    }
}
