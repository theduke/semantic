use brass::{
    component::{msg::MsgComponent, Component, Context},
    dom::{
        builder::{div, span},
        Render, Tag, TagBuilder,
    },
    effect::EffectGuard,
    signal::signal::Mutable,
};
use factordb::{query::select::Item, schema::AttrMapExt, AnyError, Id};
use semantic_core::{base::AttrUrl, plugin::ImportOutput};
use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_view::EntityView},
        form::FormHandle,
        loader::spinner,
        util::{
            buttons, notification_error, notification_success, subtitle_4, title_2, ButtonBuilder,
            Cls,
        },
    },
    context::{self, api},
    EntityRenderOpts,
};
use url::Url;

use super::import_form::{import_form_new, import_form_render, Values};

pub struct ImportPage {}

impl Render for ImportPage {
    fn render(self) -> TagBuilder {
        State::build(self)
    }
}

type Index = usize;

enum Msg {
    FormSubmit(Values),
    FetchLoaded(Result<ImportOutput, AnyError>),
    ImportLoaded(Result<Vec<Item>, AnyError>),
    ImportItem(Index),
    ImportItemLoaded {
        res: Result<Vec<Item>, AnyError>,
        url: Url,
        old: PreviewView,
    },
    PreviewClearImported,
    ImportAll,
}

#[derive(Clone)]
struct PreviewView {
    output: ImportOutput,
    item_error: Option<String>,
    imported_items: Vec<Item>,
}

enum View {
    Idle,
    Loading(EffectGuard),
    Preview(PreviewView),
    Imported(Vec<Item>),
    Error(String),
}

struct State {
    values: Values,
    form: FormHandle<Values>,
    view: Mutable<View>,
}

impl MsgComponent for State {
    type Properties = ImportPage;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: Context<Self>) -> Self {
        let handle = ctx.handle();
        Self {
            values: Values::default(),
            form: import_form_new()
                .on_submit(move |values| {
                    handle.send(Msg::FormSubmit(values.clone()));
                })
                .build(),
            view: Mutable::new(View::Idle),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FormSubmit(values) => {
                let url = if let Ok(url) = url::Url::parse(&values.url) {
                    url
                } else {
                    return;
                };

                let guard = if values.import {
                    let import_media = values.import_media;
                    ctx.spawn_map(
                        async move { api().import(url, import_media).await },
                        Msg::ImportLoaded,
                    )
                } else {
                    ctx.spawn_map(async move { api().fetch_url(url).await }, Msg::FetchLoaded)
                };

                self.form.set_loading();
                self.view.set(View::Loading(guard));
                self.values = values;
            }
            Msg::FetchLoaded(res) => {
                self.form.set_loaded();

                match res {
                    Ok(output) => {
                        self.view.set(View::Preview(PreviewView {
                            output,
                            item_error: None,
                            imported_items: Vec::new(),
                        }));
                    }
                    Err(err) => {
                        self.view.set(View::Error(err.to_string()));
                    }
                }
            }
            Msg::ImportLoaded(res) => {
                self.form.set_loaded();

                match res {
                    Ok(items) => {
                        self.view.set(View::Imported(items));
                    }
                    Err(err) => {
                        self.view.set(View::Error(err.to_string()));
                    }
                }
            }
            Msg::ImportAll => {
                if let Ok(url) = self.values.url.parse() {
                    let import_media = self.values.import_media;
                    self.view.set(View::Loading(ctx.spawn_map(
                        async move { api().import(url, import_media).await },
                        Msg::ImportLoaded,
                    )));
                }
            }
            Msg::ImportItem(index) => {
                let mut preview = {
                    match &*self.view.lock_ref() {
                        View::Preview(p) => p.clone(),
                        _ => {
                            return;
                        }
                    }
                };

                let item = if index < preview.output.items.len() {
                    preview.output.items.remove(index)
                } else {
                    tracing::warn!(%index, len=%preview.imported_items.len(), "Invalid index");
                    return;
                };

                let url = item.data.get_attr::<AttrUrl>();

                if url.is_none() {
                    tracing::warn!("no url!");
                    return;
                }

                let url = url.unwrap();

                let import_media = self.values.import_media;

                let url2 = url.clone();
                let guard = ctx.spawn(async move {
                    let res = api().import(url2, import_media).await;

                    Msg::ImportItemLoaded {
                        url,
                        res,
                        old: preview,
                    }
                });
                self.view.set(View::Loading(guard));
            }
            Msg::ImportItemLoaded { res, url, mut old } => {
                match res {
                    Ok(new_items) => {
                        let item = new_items.into_iter().find(|item| {
                            item.data
                                .get_attr::<AttrUrl>()
                                .map(|u| &u == &url)
                                .unwrap_or_default()
                        });
                        if let Some(item) = item {
                            old.imported_items.push(item);
                        }
                        old.item_error = None;
                    }
                    Err(err) => {
                        old.item_error = Some(err.to_string());
                    }
                }

                self.view.set(View::Preview(old));
            }
            Msg::PreviewClearImported => match &mut *self.view.lock_mut() {
                View::Preview(p) => {
                    p.imported_items.clear();
                    p.item_error = None;
                }
                _ => {}
            },
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let form = import_form_render(self.form.clone());

        let handle = ctx.handle();

        let content =
            self.view.signal_ref(move |view| match view {
                View::Idle => div(),
                View::Loading(_) => spinner(),
                View::Preview(PreviewView {
                    output,
                    item_error,
                    imported_items: items,
                }) => {
                    let handle = handle.clone();

                    let actions = if items.is_empty() {
                        let import_btn = ButtonBuilder::new()
                            .size_medium()
                            .label("Import all")
                            .on(handle.callback(|| Msg::ImportAll))
                            .build();

                        let clear_btn = ButtonBuilder::new()
                            .size_medium()
                            .label("Clear imported")
                            .on(handle.callback(|| Msg::PreviewClearImported))
                            .build();

                        Some(buttons().class("mb-4").and(import_btn).and(clear_btn))
                    } else {
                        None
                    };

                    let item_notification = match item_error.as_ref() {
                        Some(err) => notification_error().and(err.as_str()),
                        None if !items.is_empty() => {
                            notification_success().and(format!("Imported {} items.", items.len()))
                        }
                        None => span(),
                    };

                    let item_views = items.iter().map(|item| EntityBox {
                        item: item.clone(),
                        options: EntityRenderOpts {
                            editable: false,
                            preview: true,
                        },
                        on_delete: None,
                    });
                    let item_list = div().and(item_notification).and_iter(item_views).and(
                        if items.is_empty() {
                            None
                        } else {
                            Some(Tag::Hr.new().class("mb-4"))
                        },
                    );

                    let registry = context::registry();
                    let preview_items: Vec<_> = output
                        .items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            let mut view = EntityView::from_item(
                                &item.clone().into_db_item(),
                                &registry,
                                &EntityRenderOpts {
                                    editable: false,
                                    preview: true,
                                },
                            );

                            let handle = handle.clone();
                            let import_btn = if let Some(_url) = item.data.get_attr::<AttrUrl>() {
                                Some(
                                    ButtonBuilder::new()
                                        .label("Import")
                                        .on(move || handle.send(Msg::ImportItem(index)))
                                        .build(),
                                )
                            } else {
                                None
                            };

                            view.content = div()
                                .and(buttons().class("mb-3").and(import_btn))
                                .and(view.content);

                            view.render()
                        })
                        .collect();

                    let related_urls = if output.related_urls.is_empty() {
                        None
                    } else {
                        Some(div())
                    };

                    div()
                        .and(actions)
                        .and(item_list)
                        .and(
                            div()
                                .and(subtitle_4().and("Preview"))
                                .and_iter(preview_items),
                        )
                        .and(related_urls)
                }
                View::Imported(items) => div()
                    .and(notification_success().and(format!("Imported {} items.", items.len())))
                    .and_iter(items.iter().map(|item| {
                        EntityBox {
                            item: item.clone(),
                            options: EntityRenderOpts {
                                editable: false,
                                preview: true,
                            },
                            on_delete: None,
                        }
                        .render()
                    })),
                View::Error(err) => notification_error().and(err.as_str()),
            });

        div()
            .and(title_2().and("Import"))
            .and(form.class("mb-4").class(Cls::Box))
            .child_signal(content)
    }
}

// #[derive(Clone)]
// struct Inner {
//     url: Url,
//     is_imported: bool,
//     import_media: bool,
//     output: Option<ImportOutput>,
// }

// pub fn import_page() -> TagBuilder {
//     let loader = Loader::<Inner>::new_idle();

//     let loader2 = loader.clone();
//     let form = super::import_form::import_form(move |values| {
//         let values = values.clone();
//         let mut loader = loader2.clone();
//         Box::pin(async move {
//             let url = url::Url::parse(&values.url)?;
//             let out = api()
//                 .fetch_url(url.clone(), values.import, values.import_media)
//                 .await?;

//             loader.set_result(Ok(Inner {
//                 output: out,
//                 url,
//                 is_imported: values.import,
//                 import_media: values.import_media,
//             }));

//             Ok(())
//         })
//     });

//     let content = loader
//         .clone()
//         .signal_render(move |inner| match &inner.output {
//             None => notification_warning().and("Nothing found."),
//             Some(output) if output.items.is_empty() => notification_warning().and("Nothing found."),
//             Some(output) if !inner.is_imported => {
//                 let inner = inner.clone();
//                 let loader = loader.clone();

//                 let import_btn = ButtonBuilder::new()
//                     .size_medium()
//                     .label("Import all")
//                     .on(move || {
//                         let inner = inner.clone();
//                         loader.spawn(async move {
//                             let output = api()
//                                 .clone()
//                                 .fetch_url(inner.url.clone(), true, inner.import_media)
//                                 .await?;
//                             Ok(Inner {
//                                 output,
//                                 url: inner.url,
//                                 is_imported: true,
//                                 import_media: inner.import_media,
//                             })
//                         });
//                     })
//                     .build();

//                 let actions = buttons().class("mb-4").and(import_btn);

//                 let items: Vec<_> = output
//                     .items
//                     .iter()
//                     .map(|item| item.clone().into_db_item())
//                     .collect();
//                 let list = entity_list(
//                     &items,
//                     &context::registry(),
//                     &EntityRenderOpts {
//                         editable: false,
//                         preview: true,
//                     },
//                 );

//                 div().and(actions).and(list)
//             }
//             Some(output) => div()
//                 .and(notification_success().and(format!("Imported {} items", output.items.len())))
//                 .and_iter(output.items.iter().map(|item| {
//                     EntityBox {
//                         item: item.clone().into_db_item(),
//                         options: EntityRenderOpts {
//                             editable: false,
//                             preview: true,
//                         },
//                         on_delete: None,
//                     }
//                     .render()
//                 })),
//         });

//     div()
//         .and(title_2().and("Import"))
//         .and(form.class("mb-4").class(Cls::Box))
//         .child_signal(content)
// }
