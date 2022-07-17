use std::collections::HashMap;

use brass::{
    component::{msg::MsgComponent, Component, Context},
    dom::{builder::div, Render, TagBuilder, View},
};
use factordb::{
    prelude::{AttrMapExt, AttributeDescriptor, DataMap, Expr, Id, Item, Select},
    AnyError,
};
use semantic_core::{
    base::AttrUrl,
    plugin::{FetchUrlJob, FetchUrlOutput, ImportJob, ImportOutput},
};
use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_view::EntityView},
        form::FormHandle,
        loader::{LoadState, Loader},
        util::{
            box_, buttons, notification_error, notification_success, notification_warning,
            subtitle_4, title_2, ButtonBuilder, Cls,
        },
    },
    context::{self, api, router},
    routing::Route,
    EntityRenderOpts,
};
use url::Url;

use super::import_form::{import_form_new, import_form_render, Values};

pub struct ImportPage {
    pub url: Option<Url>,
}

impl Render for ImportPage {
    fn render(self) -> View {
        State::build(self)
    }
}

type Index = usize;

enum Msg {
    FormSubmit(Values),
    FetchLoaded(Result<(FetchUrlOutput, Vec<PreviewItem>), AnyError>),
    ImportLoaded(Result<ImportOutput, AnyError>),
    ImportItem(Index),
    ImportItemLoaded {
        res: Result<Vec<Item>, AnyError>,
        url: Url,
    },
    OpenRelatedUrl(Url),
    Clear,
    ImportAll,
}

#[derive(Clone)]
struct Preview {
    output: FetchUrlOutput,
    items: Vec<PreviewItem>,
}

#[derive(Clone)]
struct PreviewItem {
    index: Index,
    item: DataMap,
    url: Option<Url>,
    existing_id: Option<Id>,
    loader: Loader<DataMap>,
}

struct State {
    values: Values,
    form: FormHandle<Values>,

    preview: Loader<Preview>,
    full_import: Loader<Vec<Item>>,
}

async fn build_preview_items(items: Vec<DataMap>) -> Result<Vec<PreviewItem>, AnyError> {
    let old_urls: Vec<_> = items
        .iter()
        .filter_map(|item| item.get_attr::<AttrUrl>())
        .map(|url| url.to_string())
        .collect();
    let filter = Expr::in_(AttrUrl::expr(), old_urls);
    let old_page = api()
        .select(
            Select::new()
                .with_limit(items.len() as u64)
                .with_filter(filter),
        )
        .await?;
    let mut old_map: HashMap<Url, Id> = old_page
        .into_iter()
        .filter_map(|item| Some((item.get_attr::<AttrUrl>()?, item.get_id()?)))
        .collect();

    let items = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let url = item.get_attr::<AttrUrl>();
            PreviewItem {
                index,
                existing_id: url.as_ref().and_then(|url| old_map.remove(url)),
                url,
                item,
                loader: Loader::new_idle(),
            }
        })
        .collect();

    Ok(items)
}

impl MsgComponent for State {
    type Properties = ImportPage;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: Context<Self>) -> Self {
        let handle = ctx.handle();
        let mut s = Self {
            values: Values::default(),
            form: import_form_new()
                .on_submit(move |values| {
                    handle.send(Msg::FormSubmit(values.clone()));
                })
                .build(),
            preview: Loader::new_idle(),
            full_import: Loader::new_idle(),
        };

        if let Some(url) = props.url {
            // TODO: extract to helper method.
            s.update(
                Msg::FormSubmit(Values {
                    url: url.to_string(),
                    import_media: true,
                    import: false,
                }),
                ctx,
            )
        }

        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FormSubmit(values) => {
                let url = if let Ok(url) = url::Url::parse(&values.url) {
                    url
                } else {
                    return;
                };

                router().set_route_without_navigation(Route::Import {
                    url: Some(url.clone()),
                });

                if values.import {
                    let import_media = values.import_media;
                    let guard = ctx.spawn_map(
                        async move { api().import(ImportJob { url, import_media }).await },
                        Msg::ImportLoaded,
                    );
                    self.full_import.set_loading(guard);
                    self.preview.set_idle();
                } else {
                    let guard = ctx.spawn_map(
                        async move {
                            let mut out = api().fetch_url(FetchUrlJob { url }).await?;

                            let items = std::mem::take(&mut out.items)
                                .into_iter()
                                .map(|x| x.data)
                                .collect();

                            let items = build_preview_items(items).await?;

                            Ok((out, items))
                        },
                        Msg::FetchLoaded,
                    );
                    self.full_import.set_idle();
                    self.preview.set_loading(guard);
                };

                self.values = values;
                self.form.set_loading();
            }
            Msg::FetchLoaded(res) => {
                self.form.set_loaded();

                self.preview.set_idle();

                match res {
                    Ok((output, items)) => {
                        let preview = Preview { output, items };
                        self.preview.set_result(Ok(preview));
                    }
                    Err(err) => {
                        self.preview.set_err(err);
                    }
                }
            }
            Msg::ImportLoaded(res) => {
                self.form.set_loaded();
                self.full_import.set_result(res.map(|out| out.items));
                self.preview.set_idle();
            }
            Msg::ImportAll => {
                if let Ok(url) = self.values.url.parse() {
                    let import_media = self.values.import_media;
                    let guard = ctx.spawn_map(
                        async move { api().import(ImportJob { url, import_media }).await },
                        Msg::ImportLoaded,
                    );
                    self.full_import.set_loading(guard);
                    self.preview.set_idle();
                }
            }
            Msg::ImportItem(index) => {
                let item = match &*self.preview.get().lock_ref() {
                    LoadState::Success(preview) => preview.items.get(index).cloned(),
                    _ => None,
                };
                let item = if let Some(x) = item {
                    x
                } else {
                    return;
                };

                let url = if let Some(x) = item.item.get_attr::<AttrUrl>() {
                    x
                } else {
                    return;
                };

                let import_media = self.values.import_media;

                let url2 = url.clone();
                let guard = ctx.spawn(async move {
                    let res = api()
                        .import(ImportJob {
                            url: url2,
                            import_media,
                        })
                        .await;

                    Msg::ImportItemLoaded {
                        url,
                        res: res.map(|out| out.items),
                    }
                });
                item.loader.set_loading(guard);
            }
            Msg::ImportItemLoaded { res, url } => match &mut *self.preview.get().lock_mut() {
                LoadState::Success(ref mut preview) => {
                    let item_opt = preview
                        .items
                        .iter_mut()
                        .find(|item| item.url.as_ref() == Some(&url));

                    let item = if let Some(x) = item_opt {
                        x
                    } else {
                        return;
                    };

                    match res {
                        Ok(items) => {
                            let new_item = items.into_iter().find(|new_item| {
                                new_item.data.get_attr::<AttrUrl>().as_ref() == Some(&url)
                            });
                            if let Some(new) = new_item {
                                item.loader.set_result(Ok(new.data));
                            } else {
                                item.loader.set_err("Could not import item.");
                            }
                        }
                        Err(err) => item.loader.set_err(err),
                    }
                }
                _ => {}
            },
            Msg::Clear => {
                self.preview.set_idle();
                self.full_import.set_idle();
            }
            Msg::OpenRelatedUrl(url) => {
                // TODO: refactor above FormSubmit handler into a helper function
                self.update(
                    Msg::FormSubmit(Values {
                        url: url.to_string(),
                        import_media: self.values.import_media,
                        import: false,
                    }),
                    ctx,
                );
            }
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let form = import_form_render(self.form.clone());

        let handle = ctx.handle();

        let preview = self.preview.signal_render(move |preview| -> View {
            if preview.items.is_empty() {
                return notification_warning().and("Nothing found").into();
            }

            let actions = {
                let import_btn = ButtonBuilder::new()
                    .size_medium()
                    .label("Import all")
                    .on(handle.callback(|| Msg::ImportAll))
                    .build();

                let clear_btn = ButtonBuilder::new()
                    .size_medium()
                    .label("Clear")
                    .on(handle.callback(|| Msg::Clear))
                    .build();

                buttons().class("mb-4").and(import_btn).and(clear_btn)
            };

            let handle = handle.clone();

            let mut items = div();

            for item2 in preview.items.clone() {
                let handle = handle.clone();

                let item = item2.clone();
                let view = item2.loader.signal_render_state(move |state| {
                    let registry = context::registry();
                    let handle = handle.clone();

                    match state {
                        LoadState::Idle => {
                            let mut view = EntityView::from_item(
                                &item.item.clone(),
                                &registry,
                                &EntityRenderOpts {
                                    editable: false,
                                    preview: true,
                                },
                            );

                            let handle = handle.clone();
                            let import_btn = if item.url.is_some() {
                                let index = item.index;
                                Some(
                                    ButtonBuilder::new()
                                        .label("Import")
                                        .on(move || handle.send(Msg::ImportItem(index)))
                                        .build(),
                                )
                            } else {
                                None
                            };

                            let exists_warning = item
                                .existing_id
                                .as_ref()
                                .map(|_| notification_warning().and("Item is already imported"));

                            view.content = div()
                                .and(buttons().class("mb-3").and(import_btn))
                                .and(exists_warning)
                                .and(view.content);

                            view.render()
                        }
                        LoadState::Loading(_) => {
                            let mut view = EntityView::from_item(
                                &item.item.clone(),
                                &registry,
                                &EntityRenderOpts {
                                    editable: false,
                                    preview: true,
                                },
                            );

                            let import_btn = ButtonBuilder::new().label("Import").loading().build();

                            view.content = div()
                                .and(buttons().class("mb-3").and(import_btn))
                                .and(view.content);

                            view.render()
                        }
                        LoadState::Success(item) => {
                            let box_ = EntityBox {
                                item: item.clone(),
                                options: EntityRenderOpts {
                                    editable: false,
                                    preview: true,
                                },
                                show_link: true,
                                on_delete: None,
                            };

                            div()
                                .style_raw("border-right: 10px solid green; padding-right: 10px;")
                                .and(box_.render())
                                .into()
                        }
                        LoadState::Failed(err) => {
                            let mut view = EntityView::from_item(
                                &item.item.clone(),
                                &registry,
                                &EntityRenderOpts {
                                    editable: false,
                                    preview: true,
                                },
                            );

                            view.content =
                                div().and(notification_error().and(err)).and(view.content);

                            view.render()
                        }
                    }
                });
                items.add_signal(view);
            }

            let load_more = if let Some(link) = &preview.output.load_more_url {
                let url = link.url.clone();
                let elem = div()
                    .class("mb-4")
                    .style_raw("display: flex; justify-content: center")
                    .and(
                        ButtonBuilder::new()
                            .label(&link.label)
                            .size_large()
                            .on(handle.callback(move || Msg::OpenRelatedUrl(url.clone())))
                            .build(),
                    );
                Some(elem)
            } else {
                None
            };

            let related_urls = if preview.output.related_urls.is_empty() {
                None
            } else {
                let handle = handle.clone();
                let links = preview.output.related_urls.iter().map(move |link| {
                    let handle = handle.clone();
                    let url = link.url.clone();
                    ButtonBuilder::new()
                        .label(&link.label)
                        .on(handle.callback(move || Msg::OpenRelatedUrl(url.clone())))
                        .build()
                });
                Some(
                    box_()
                        .and(subtitle_4().and("Links"))
                        .and(buttons().and_iter(links)),
                )
            };

            div()
                .and(actions)
                .and(items)
                .and(load_more)
                .and(related_urls)
                .into()
        });

        let full = self.full_import.signal_render(|items| {
            div()
                .and(notification_success().and(format!("Imported {} items.", items.len())))
                .and_iter(items.iter().map(|item| {
                    EntityBox {
                        item: item.data.clone(),
                        show_link: true,
                        options: EntityRenderOpts {
                            editable: false,
                            preview: true,
                        },
                        on_delete: None,
                    }
                    .render()
                }))
                .into()
        });

        div()
            .and(title_2().and("Import"))
            .and(form.class("mb-4").class(Cls::Box))
            .signal(preview)
            .signal(full)
    }
}
