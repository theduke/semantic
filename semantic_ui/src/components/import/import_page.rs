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

    submit: brass::Callback<url::Url>,

    registry: semantic_ui_core::SharedRegistry,

    guard: Option<brass::EffectGuard>,
}

pub enum Msg {
    FormSubmit(url::Url),
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
            submit: ctx.callback_map(Msg::FormSubmit),
            guard: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::FormSubmit(url) => {
                if self.import_load.is_loading() {
                    return;
                }
                self.persist_load.set_idle();
                match self.registry.find_importer(url.as_str()) {
                    Some(plugin) => {
                        self.import_load.set_loading();
                        let f = plugin.import(url, &crate::api());

                        self.guard = Some(ctx.run_map(f, Msg::ImporterLoaded));
                    }
                    None => {
                        self.import_load
                            .set_failed("No importer for the given URL found");
                    }
                }
            }
            Msg::ImporterLoaded(res) => {
                tracing::trace!(?res, "import result");
                self.import_load.set_result(res);
            }
            Msg::ImportAll => {
                if self.is_loading() {
                    return;
                }
                if let LoadState::Success(page) = &self.import_load {
                    let items = page.items.clone();
                    // TODO: toggle for item import.
                    let f = async move {
                        let api = crate::api();
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
            on_submit: self.submit.clone(),
        };
        let form_wrap = div_with(form).class("box");
        let loader1 = self.import_load.render(|page| {
            let rendered_page = super::super::entity::entity_page(
                page,
                &self.registry,
                &EntityRenderOpts { editable: false },
            );

            let already_imported = self.persist_load.is_success();
            let btn_label = if self.persist_load.is_loading() {
                "..."
            } else {
                "Import All"
            };
            let import_button = div().class("mb-2").and(
                brass_bulma::button_medium()
                    .and(btn_label)
                    .attr_toggle_if(already_imported, Attr::Disabled)
                    .on(
                        Event::Click,
                        _ctx.callback_ignore_event(|| Msg::ImportAll),
                    ),
            );
            div().and(import_button).and(rendered_page).build()
        });
        let loader2 = self
            .persist_load
            .render(|_| brass_bulma::notification_success("Import succeeded.").build());


        let header = brass_bulma::h2_with("Import");

        div().and((header, form_wrap, loader2, loader1)).build()
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
