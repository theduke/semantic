use brass::{
    dom::{Attr, Event},
    vdom::{div, div_with},
};
use brass_bulma;

use factordb::{query::select::ItemPage, AnyError};
use semantic_ui_core::{ContextExt, EntityRenderOpts};

use semantic_ui_core::loader::LoadState;

pub struct ImportPage {
    import_load: LoadState<ItemPage>,
    persist_load: LoadState<()>,

    on_preview: brass::Callback<url::Url>,
    on_import: brass::Callback<url::Url>,

    registry: semantic_ui_core::SharedRegistry,

    auto_import: bool,
    guard: Option<brass::EffectGuard>,
}

#[derive(Debug)]
pub enum Msg {
    FormSubmitPreview(url::Url),
    FormSubmitImport(url::Url),
    ImporterLoaded(Result<ItemPage, AnyError>),
    ImportAll,
    ImportLoaded(Result<(), AnyError>),
}

impl ImportPage {
    fn is_loading(&self) -> bool {
        self.import_load.is_loading() || self.persist_load.is_loading()
    }
}

impl brass::Component for ImportPage {
    type Properties = ();
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            import_load: LoadState::Idle,
            persist_load: LoadState::Idle,
            registry: ctx.registry().clone(),
            on_preview: ctx.callback_map(Msg::FormSubmitPreview),
            on_import: ctx.callback_map(Msg::FormSubmitImport),
            auto_import: false,
            guard: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        tracing::trace!(?msg);
        match msg {
            Msg::FormSubmitPreview(url) => {
                if self.import_load.is_loading() {
                    return;
                }
                self.persist_load.set_idle();
                match self.registry.find_importer(url.as_str()) {
                    Some(plugin) => {
                        self.import_load.set_loading();
                        let f = plugin.import(url, ctx.api());

                        self.guard = Some(ctx.run_map(f, Msg::ImporterLoaded));
                    }
                    None => {
                        self.import_load
                            .set_failed("No importer for the given URL found");
                    }
                }
            }
            Msg::FormSubmitImport(url) => {
                self.auto_import = true;
                self.update(Msg::FormSubmitPreview(url), ctx);
            }
            Msg::ImporterLoaded(res) => {
                self.import_load.set_result(res);
                if self.auto_import {
                    self.update(Msg::ImportAll, ctx);
                }
            }
            Msg::ImportAll => {
                if self.is_loading() {
                    return;
                }
                if let LoadState::Success(page) = &self.import_load {
                    let items = page.items.clone();
                    // TODO: toggle for item import.
                    let api = ctx.api().clone();
                    let f = async move {
                        api.import(items, true).await?;
                        Ok(())
                    };
                    self.guard = Some(ctx.run_map(f, Msg::ImportLoaded));
                    self.persist_load.set_loading();
                }
            }
            Msg::ImportLoaded(res) => {
                self.persist_load.set_result(res);
            }
        }
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        let form = super::import_form::ImportFormProps {
            loading: self.import_load.is_loading(),
            on_preview: self.on_preview.clone(),
            on_import: self.on_import.clone(),
        };
        let form_wrap = div_with(form).class("box");
        let loader1 = self.import_load.render(|page| {
            let rendered_page = super::super::entity::entity_list(
                page,
                &self.registry,
                &EntityRenderOpts {
                    editable: false,
                    preview: true,
                },
            );

            let already_imported = self.persist_load.is_success();
            tracing::trace!(?already_imported);
            let btn_label = if self.persist_load.is_loading() {
                "..."
            } else {
                "Import All"
            };
            let import_toggle = if already_imported {
                brass_bulma::notification(
                    brass_bulma::Color::Success,
                    format!("Imported {} entities.", page.items.len()),
                )
            } else {
                div().class("mb-2").and(
                    brass_bulma::button_large()
                        .and(btn_label)
                        .attr_toggle_if(self.persist_load.is_loading(), Attr::Disabled)
                        .on(Event::Click, _ctx.on_simple(|| Msg::ImportAll)),
                )
            };
            div().and((import_toggle, rendered_page)).build()
        });

        let header = brass_bulma::h2_with("Import");
        tracing::trace!(?loader1);

        div().and((header, form_wrap, loader1)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }

    fn build(props: Self::Properties) -> brass::VNode {
        brass::vdom::component::<Self>(props)
    }
}
