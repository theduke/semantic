use brass::{
    vdom::{div, div_with, Render},
    Callback, VNode,
};
use factordb::{query::select::Item, schema::AttrMapExt, AnyError};
use semantic_ui_core::{
    loader::LoadState, ContextExt, DynEntityRenderer, EntityInfo, EntityRenderOpts,
    RenderContextExt,
};

use super::entity_title;

pub struct EntityView {
    pub item: Item,
    pub options: EntityRenderOpts,
    pub on_delete: Option<Callback<Item>>,
}

enum Msg {
    ToggleActions,
    Open,
    DeleteStart,
    DeleteConfirm,
    DeleteCancel,
    DeleteLoaded(Result<(), AnyError>),
}

enum Action {
    Delete(LoadState<()>),
}

struct EntityViewComponent {
    item: Item,
    options: EntityRenderOpts,
    on_delete: Option<Callback<Item>>,

    actions_active: bool,
    title: String,
    type_name: Option<String>,
    content_renderer: Option<DynEntityRenderer>,
    info: Option<EntityInfo>,

    active_action: Option<Action>,
}

brass::enable_props!(EntityView => EntityViewComponent);

impl brass::Component for EntityViewComponent {
    type Properties = EntityView;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let item = props.item;
        let options = props.options;

        let registry = ctx.registry();
        let title = entity_title(&item.data);

        let info = item
            .data
            .get_type_name()
            .and_then(|name| registry.entity(name));
        let type_name = super::entity_type_name(&item.data, info);
        let content_renderer = info
            .and_then(|info| registry.entity_content_renderer(&info.schema.ident))
            .cloned();

        Self {
            item,
            options,
            on_delete: props.on_delete,
            title,
            type_name,
            actions_active: false,
            content_renderer,
            info: info.cloned(),
            active_action: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::ToggleActions => {
                self.actions_active = !self.actions_active;
            }
            Msg::Open => {
                if let Some(id) = self.item.data.get_id() {
                    ctx.router()
                        .goto(semantic_ui_core::router::Route::Entity(id.into()));
                }
            }
            Msg::DeleteStart => {
                self.active_action = Some(Action::Delete(LoadState::Idle));
            }
            Msg::DeleteConfirm => {
                if let Some(id) = self.item.data.get_id() {
                    let future = async move {
                        crate::api()
                            .mutate(factordb::query::mutate::Mutate::delete(id))
                            .await
                    };
                    let guard = ctx.run_map(future, Msg::DeleteLoaded);

                    self.active_action = Some(Action::Delete(LoadState::Loading(Some(guard))));
                }
            }
            Msg::DeleteCancel => {
                self.active_action = None;
            }
            Msg::DeleteLoaded(res) => {
                if res.is_ok() {
                    if let Some(callback) = &self.on_delete {
                        callback.send(self.item.clone());
                        return;
                    } else {
                        ctx.router().goto(semantic_ui_core::router::Route::Browse);
                    }
                } else {
                    self.active_action = Some(Action::Delete(res.into()));
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        // Header.
        let title = brass_bulma::card_header_title(&self.title)
            .style_raw("flex-grow: 0; cursor: pointer;")
            .on_click(ctx.on_simple(|| Msg::Open));
        let ty = self
            .type_name
            .as_ref()
            .map(|name| {
                div()
                    .and(name)
                    .class("is-flex is-align-items-center mr-3")
                    .build()
            })
            .unwrap_or(VNode::Empty);

        let mut action_buttons = Vec::new();

        if self.options.editable {
            let delete_btn = brass_bulma::button()
                .and(brass_bulma::icon_fa("fas fa-trash"))
                .on_click(ctx.on_simple(|| Msg::DeleteStart));
            action_buttons.push(delete_btn.render());
        }
        let actions = brass_bulma::buttons().and_iter(action_buttons);

        let actions_dropdown = brass_bulma::Dropdown {
            trigger: brass_bulma::icon_fa("fas fa-cog"),
            content: "hello",
            is_hoverable: true,
            is_active: self.actions_active,
            on_toggle: ctx.on_simple(|| Msg::ToggleActions),
        };
        let hover_actions = div().and(actions_dropdown).style_raw("margin-left: auto;");

        let header = brass_bulma::card_header()
            .and((title, ty, actions, hover_actions))
            .build();

        let active_action = match &self.active_action {
            Some(Action::Delete(loader)) => {
                let confirm = brass_bulma::button()
                    .and_class(brass_bulma::Color::Danger.as_class())
                    .and("Really Delete")
                    .attr_toggle_if(
                        loader.is_loading() || loader.is_success(),
                        brass::dom::Attr::Disabled,
                    )
                    .on_click(ctx.on_simple(|| Msg::DeleteConfirm));

                let state = loader.render(|_| brass_bulma::notification_error("Deleted!").build());

                div_with((confirm, state)).build()
            }
            None => VNode::Empty,
        };

        let content = if let Some(renderer) = &self.content_renderer {
            renderer(&self.item, &self.options)
        } else {
            super::entity_fields_table(&self.item.data, self.info.as_ref(), ctx.registry()).build()
        };

        let card_content = brass_bulma::card_content().and((active_action, content));
        brass_bulma::card()
            .and_class("mb-4")
            .and((header, card_content))
            .build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        self.on_delete = props.on_delete;
        if props.item != self.item || props.options != self.options {
            self.item = props.item;
            self.options = props.options;

            true
        } else {
            false
        }
    }
}
