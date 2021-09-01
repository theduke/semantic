use brass::{
    dom::Attr,
    vdom::{div, div_with, s, Render},
    Callback, Str, VNode,
};
use factordb::{query::select::Item, schema::AttrMapExt, AnyError, Id};
use semantic_ui_core::{
    loader::LoadState, ContextExt, DynEntityRenderer, EntityInfo, EntityRenderOpts,
    RenderContextExt,
};

use crate::components::base::{collections::EntityCollectionManager, tags::EntityTagManager};

use super::{entity_joins, entity_title};

pub struct EntityView {
    pub title: String,
    pub type_name: Option<String>,
    pub on_open: Option<Callback<()>>,
    pub actions: Vec<EntityActionButton>,
    pub content: VNode,
}

impl EntityView {
    pub fn build(
        item: &Item,
        opts: &EntityRenderOpts,
        registry: &semantic_ui_core::Registry,
    ) -> Self {
        let info = registry.entity_by_item(item);

        let title = super::entity_title(&item.data);
        let type_name = super::entity_type_name(&item.data, info);

        let content = if let Some(renderer) =
            info.and_then(|info| registry.entity_content_renderer(&info.schema.ident))
        {
            renderer(item, opts)
        } else {
            super::entity_fields_table(&item.data, info, registry).build()
        };

        Self {
            title,
            type_name,
            on_open: None,
            actions: Vec::new(),
            content,
        }
    }
}

impl Render for EntityView {
    fn render(self) -> VNode {
        // Header.
        let title = {
            let t = brass_bulma::card_header_title(self.title)
                .style_raw(s("flex-grow: 0; cursor: pointer;"));
            if let Some(on) = self.on_open {
                t.on_click(on.on(|_| ()))
            } else {
                t
            }
        };

        let ty = self
            .type_name
            .map(|name| {
                div()
                    .and(name)
                    .class(s("is-flex is-align-items-center mr-3"))
                    .build()
            })
            .unwrap_or(VNode::Empty);

        let actions = brass_bulma::buttons()
            .style_raw(s("margin: 0;"))
            .and_iter(self.actions);

        let header = brass_bulma::card_header().and((title, ty, actions));

        let card_content = brass_bulma::card_content().and(self.content);
        brass_bulma::card()
            .and_class("mb-4")
            .and((header, card_content))
            .build()
    }
}

pub struct EntityBox {
    pub item: Item,
    pub options: EntityRenderOpts,
    pub on_delete: Option<Callback<Item>>,
}

enum Msg {
    ToggleActions,
    ToggleShowTable,
    ToggleCollectionManager,
    ToggleTagManager,
    OpenSourceUrl,
    Open,
    DeleteStart,
    DeleteConfirm,
    DeleteCancel,
    DeleteLoaded(Result<(), AnyError>),
}

enum Action {
    Delete(LoadState<()>),
    ManageCollections,
    ManageTags,
}

pub struct EntityActionButton {
    icon: Str,
    label: Str,
    is_active: bool,
    is_disabled: bool,
    on: Callback<()>,
}

impl brass::vdom::Render for EntityActionButton {
    fn render(self) -> VNode {
        brass_bulma::button_small()
            .and_class_if(self.is_active, "is-active")
            .attr(Attr::Title, self.label)
            .attr_toggle_if(self.is_disabled, Attr::Disabled)
            .style_raw(s("margin: 0"))
            .and(brass_bulma::icon_fa(self.icon))
            .on_click(self.on.on(|_| ()))
            .build()
    }
}

struct State {
    item: Item,
    entity_id: Option<Id>,
    options: EntityRenderOpts,
    on_delete: Option<Callback<Item>>,

    actions_active: bool,
    title: String,
    type_name: Option<String>,
    content_renderer: Option<DynEntityRenderer>,
    info: Option<EntityInfo>,

    show_table: bool,

    active_action: Option<Action>,
}

impl State {
    fn is_deleting(&self) -> bool {
        match &self.active_action {
            Some(Action::Delete(_)) => true,
            _ => false,
        }
    }
}

brass::enable_props!(EntityBox => State);

impl brass::Component for State {
    type Properties = EntityBox;
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
            entity_id: item.data.get_id(),
            item,
            options,
            on_delete: props.on_delete,
            title,
            type_name,
            actions_active: false,
            content_renderer,
            info: info.cloned(),
            show_table: false,
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
                        .goto(semantic_ui_core::routing::Route::Entity(id.into()));
                }
            }
            Msg::OpenSourceUrl => {
                if let Some(url) = self.item.data.get_attr::<semantic_core::base::AttrUrl>() {
                    let _ = brass::util::window()
                        .open_with_url_and_target(url.as_str(), "_blank")
                        .map_err(|_err| {
                            tracing::error!("Could not open window");
                        });
                }
            }
            Msg::DeleteStart => {
                self.active_action = Some(Action::Delete(LoadState::Idle));
            }
            Msg::DeleteConfirm => {
                if let Some(id) = self.item.data.get_id() {
                    let api = ctx.api().clone();
                    let future = async move {
                        api.mutate(factordb::query::mutate::Mutate::delete(id))
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
                        ctx.router().goto(semantic_ui_core::routing::Route::Browse);
                    }
                } else {
                    self.active_action = Some(Action::Delete(res.into()));
                }
            }
            Msg::ToggleShowTable => {
                self.show_table = !self.show_table;
            }
            Msg::ToggleCollectionManager => {
                if self
                    .active_action
                    .as_ref()
                    .map(|x| matches!(x, Action::ManageCollections))
                    .unwrap_or_default()
                {
                    self.active_action = None;
                } else {
                    self.active_action = Some(Action::ManageCollections);
                }
            }
            Msg::ToggleTagManager => {
                if self
                    .active_action
                    .as_ref()
                    .map(|x| matches!(x, Action::ManageTags))
                    .unwrap_or_default()
                {
                    self.active_action = None;
                } else {
                    self.active_action = Some(Action::ManageTags);
                }
            }
        }
    }

    fn render(&self, mut ctx: brass::RenderContext<Self>) -> brass::VNode {
        let mut actions = Vec::new();

        if self.item.data.has_attr::<semantic_core::base::AttrUrl>() {
            actions.push(EntityActionButton {
                icon: s("fas fa-globe"),
                label: s("Go to URL"),
                is_active: false,
                is_disabled: false,
                on: ctx.callback_map(|_: ()| Msg::OpenSourceUrl),
            })
        }

        actions.push(EntityActionButton {
            icon: s("fas fa-table"),
            label: s("Show Table"),
            is_active: self.show_table || self.content_renderer.is_none(),
            is_disabled: self.content_renderer.is_none(),
            on: ctx.callback_map(|_: ()| Msg::ToggleShowTable),
        });

        actions.push(EntityActionButton {
            icon: s("fas fa-list"),
            label: s("Manage Collections"),
            is_active: false,
            is_disabled: false,
            on: ctx.callback_map(|_: ()| Msg::ToggleCollectionManager),
        });

        actions.push(EntityActionButton {
            icon: s("fas fa-tags"),
            label: s("Manage Tags"),
            is_active: false,
            is_disabled: false,
            on: ctx.callback_map(|_: ()| Msg::ToggleTagManager),
        });

        if self.options.editable {
            let is_deleting = self.is_deleting();
            actions.push(EntityActionButton {
                icon: s("fas fa-trash"),
                label: s("Delete"),
                is_active: is_deleting,
                is_disabled: is_deleting,
                on: ctx.callback_map(|_: ()| Msg::DeleteStart),
            });
        }

        let active_action = match &self.active_action {
            Some(Action::Delete(loader)) => {
                let confirm = brass_bulma::button()
                    .and_class(brass_bulma::Color::Danger.as_class())
                    .and(s("Really Delete"))
                    .attr_toggle_if(loader.is_loading() || loader.is_success(), Attr::Disabled)
                    .on_click(ctx.on_simple(|| Msg::DeleteConfirm));

                let state = loader.render(|_| brass_bulma::notification_error("Deleted!").build());

                div_with((confirm, state)).build()
            }
            Some(Action::ManageCollections) => {
                if let Some(id) = self.entity_id {
                    let manager = EntityCollectionManager { entity_id: id };
                    let content = brass_bulma::box_().and(manager);
                    brass_bulma::modal(content, ctx.callback_map(|_| Msg::ToggleCollectionManager))
                        .build()
                } else {
                    VNode::Empty
                }
            }
            Some(Action::ManageTags) => {
                if let Some(id) = self.entity_id {
                    let manager = EntityTagManager { entity_id: id };
                    let content = brass_bulma::box_().and(manager);
                    brass_bulma::modal(content, ctx.callback_map(|_| Msg::ToggleTagManager)).build()
                } else {
                    VNode::Empty
                }
            }
            None => VNode::Empty,
        };

        let registry = ctx.registry();

        let content = match &self.content_renderer {
            Some(renderer) if !self.show_table => renderer(&self.item, &self.options),
            _ => super::entity_fields_table(&self.item.data, self.info.as_ref(), registry).build(),
        };

        let joins = entity_joins(&self.item, &self.options, registry);
        let content = div_with((active_action, content, joins)).build();

        EntityView {
            title: self.title.clone(),
            type_name: self.type_name.clone(),
            on_open: Some(ctx.callback_map(|_: ()| Msg::Open)),
            actions,
            content,
        }
        .render()
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
