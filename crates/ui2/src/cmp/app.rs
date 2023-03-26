use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_router::{Route, Router};
use factdb::DataMap;
use semantic_core::api::SemanticSchema;

use super::load::{render_future_res, Loader};

pub fn App<'a>(cx: Scope<'a>) -> Element {
    let api = crate::api::new_api(Some("http://localhost:3000/api/query".to_string()));

    {
        let api = api.clone();
        use_shared_state_provider(cx, || api);
    }

    let fut = use_future(cx, (), move |_| async move { api.schema().await });

    render_future_res(cx, fut, |data| {
        use_shared_state_provider::<SemanticSchema>(cx, move || data.clone());
        cx.render(rsx! {
            AppRouter {}
        })
    })
}

fn AppRouter(cx: Scope<'_>) -> Element {
    cx.render(rsx! {
        Router {
            div {
                class: "container",
                Route {
                    to: "/",
                    BrowserPage {}
                }
            }
        }
    })
}

fn BrowserPage(cx: Scope<'_>) -> Element {
    cx.render(rsx! {
        div {
            class: "card",
            div {
                class: "card-header",
                "Filter"
            }
            div {
                class: "card-body",
                div {
                }
            }
        }
    })
}

#[inline_props]
fn Browser(cx: Scope<'_>) -> Element {
    use futures::StreamExt;

    let status = use_state::<Loader<Vec<DataMap>, anyhow::Error>>(cx, || Loader::Idle);

    let api = use_shared_state::<crate::api::Api>(cx).unwrap();

    use_coroutine(cx, move |mut rx: UnboundedReceiver<EntityFilter>| {
        let status = status.to_owned();
        async move {
            while let Some(filter) = rx.next().await {
                status.set(Loader::Loading);
                let sel = factdb::Select {
                    ..Default::default()
                };
                let res = crate::api::new_api(None).select(sel).await;
                status.set(Loader::from_result(res));
            }
        }
    });

    cx.render(rsx! {
        div {

        }
    })
}

pub struct EntityFilter {
    pub search: String,
}

pub struct EntityQuery {
    pub filter: EntityFilter,
}

#[derive(Props)]
struct EntityFilterFormProps {
    on_update: Arc<dyn Fn(EntityFilter)>,
}

impl PartialEq for EntityFilterFormProps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.on_update, &other.on_update)
    }
}

fn EntityFilterForm(cx: Scope<'_, EntityFilterFormProps>) -> Element {
    let search = use_state(cx, || String::new());

    cx.render(rsx! {
        div {
            div {
                class: "input-group mb-3",
                span {
                    class: "input-group-text",
                    "Search"
                }
                input {
                    r#type: "text",
                    class: "form-control",
                    value: "{search}",
                    oninput: move |e| {
                        search.set(e.value.clone());
                    }
                }
            }

            div {
                button {
                    class: "btn btn-primary",
                    onclick: |_| {
                        (cx.props.on_update)(EntityFilter {
                            search: search.get().clone(),
                        });
                    },
                    "Search",
                }
            }
        }
    })
}
