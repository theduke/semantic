use brass::dom::{builder::div, TagBuilder};
use factordb::AnyError;
use semantic_core::plugin::ImportOutput;
use semantic_ui_core::{
    components::{
        entity::entity_list,
        loader::{LoadState, Loader},
        util::{box_, notification_error, notification_success, title_2, ButtonBuilder},
    },
    context, EntityRenderOpts,
};

use super::import_form::Values;

pub struct ImportPage {}

impl brass::dom::Render for ImportPage {
    fn render(self) -> TagBuilder {
        brass::component::build_component::<State>(self)
    }
}

struct State {
    preview_load: Loader<Option<ImportOutput>>,
    persist_load: Loader<()>,
    is_preview: bool,
}

pub enum Msg {
    FormSubmit(Values),
    ImportAll,
    ImportLoaded(Result<(), AnyError>),
}

impl State {
    fn is_loading(&self) -> bool {
        self.preview_load.is_loading() || self.persist_load.is_loading()
    }
}

impl brass::component::msg::MsgComponent for State {
    type Properties = ImportPage;
    type Msg = Msg;

    fn init(_props: Self::Properties, _ctx: brass::component::Context<'_, Self>) -> Self {
        Self {
            preview_load: Loader::new_idle(),
            persist_load: Loader::new_idle(),
            is_preview: false,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::FormSubmit(values) => {
                if self.persist_load.is_loading() || self.preview_load.is_loading() {
                    return;
                }
                if let Ok(url) = url::Url::parse(&values.url) {
                    self.persist_load.set_idle();
                    self.is_preview = !values.import;

                    let api = context::api();
                    let f = async move {
                        api.fetch_url(url.clone(), values.import, values.import_media)
                            .await
                    };
                    self.preview_load.spawn(f);
                }
            }
            Msg::ImportAll => {
                if self.is_loading() || !self.is_preview {
                    return;
                }
                if let LoadState::Success(Some(output)) = &*self.preview_load.get().lock_ref() {
                    let items = output.items.clone();
                    // TODO: toggle for item import.
                    let api = context::api();
                    let f = async move {
                        api.import(items, true).await?;
                        Ok(())
                    };
                    let guard = ctx.spawn_map(f, Msg::ImportLoaded);
                    self.persist_load.set_loading(guard);
                }
            }
            Msg::ImportLoaded(res) => {
                self.persist_load.set_result(res);
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> TagBuilder {
        let handle = ctx.handle();
        let form = super::import_form::import_form(move |values| {
            handle.send(Msg::FormSubmit(values.clone()));
        });

        let form_wrap = box_().and(form);

        let handle = ctx.handle();
        let persist_load = self.persist_load.clone();
        let loader1 = self.preview_load.signal_render(move |opt_res| {
            if let Some(output) = opt_res {
                let rendered_page = entity_list(
                    &output.items,
                    &context::registry(),
                    &EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                );

                let handle = handle.clone();
                let persist_loader = persist_load.signal_render_state(move |state| match state {
                    LoadState::Idle => ButtonBuilder::new()
                        .size_large()
                        .label("Import All")
                        .on(handle.callback(|| Msg::ImportAll))
                        .build(),
                    LoadState::Loading(_) => ButtonBuilder::new()
                        .size_large()
                        .label("Import All")
                        .loading()
                        .build(),
                    LoadState::Success(_) => notification_success().and("Import successful."),
                    LoadState::Failed(err) => {
                        notification_error().and("Could not import: ").and(err)
                    }
                });

                div().child_signal(persist_loader).and(rendered_page)
            } else {
                notification_error().and("No suitable importer found")
            }
        });

        let header = title_2().and("Import");

        div().and((header, form_wrap)).child_signal(loader1)
    }
}
