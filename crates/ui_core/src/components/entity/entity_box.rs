use brass::{
    component::{msg::MsgComponent, Context},
    dom::{builder::div, Render, TagBuilder},
    signal::signal::{Mutable, SignalExt},
    DomStr,
};
use factordb::{query::select::Item, schema::AttrMapExt};

use crate::{EntityRenderOpts, base::{collection::entity_collection_manager, tags::entity_tag_manager}, components::util::modal::modal, context, routing::Route};

use super::{
    entity_deleter::EntityDeleter,
    entity_view::{EntityActionButton, EntityView},
};

pub struct EntityBox {
    pub item: Item,
    pub options: EntityRenderOpts,
    pub on_delete: Option<Box<dyn Fn(Item)>>,
}

impl Render for EntityBox {
    fn render(self) -> TagBuilder {
        brass::component::build_component::<State>(self)
    }
}

enum Msg {
    ToggleActions,
    Deleted,
    ClearAction,
    ToggleShowTable,
    ToggleCollectionManager,
    ToggleTagManager,
    OpenSourceUrl,
    Open,
    DeleteStart,
}

#[derive(Clone)]
enum Action {
    Delete,
    ManageCollections,
    ManageTags,
}

struct State {
    item: Item,
    // entity_id: Option<Id>,
    options: EntityRenderOpts,
    on_delete: Option<Box<dyn Fn(Item)>>,
    show_table: Mutable<bool>,
    action: Mutable<Option<Action>>,
}

impl State {
    fn is_deleting(&self) -> bool {
        matches!(&*self.action.lock_ref(), Some(Action::Delete))
    }
}

impl MsgComponent for State {
    type Properties = EntityBox;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: Context<'_, Self>) -> Self {
        Self {
            item: props.item,
            on_delete: props.on_delete,
            options: props.options,
            show_table: Mutable::new(false),
            action: Mutable::new(None),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: Context<'_, Self>) {
        match msg {
            Msg::ToggleActions => {
                todo!()
            }
            Msg::Open => {
                if let Some(id) = self.item.data.get_id() {
                    context::router().goto(Route::Entity(id.into()));
                }
            }
            Msg::OpenSourceUrl => {
                if let Some(url) = self.item.data.get_attr::<semantic_core::base::AttrUrl>() {
                    let _ = brass::web::window()
                        .open_with_url_and_target(url.as_str(), "_blank")
                        .map_err(|_err| {
                            tracing::error!("Could not open window");
                        });
                }
            }
            Msg::DeleteStart => {
                self.action.set(Some(Action::Delete));
            }
            Msg::ToggleShowTable => {
                self.show_table.replace_with(|old| !*old);
            }
            Msg::ToggleCollectionManager => {
                self.action.replace_with(|old| {
                    if matches!(old, Some(Action::ManageCollections)) {
                        None
                    } else {
                        Some(Action::ManageCollections)
                    }
                });
            }
            Msg::ToggleTagManager => {
                tracing::trace!("toggling tag manager");
                self.action.replace_with(|old| {
                    if matches!(old, Some(Action::ManageTags)) {
                        None
                    } else {
                        Some(Action::ManageTags)
                    }
                });
            }
            Msg::Deleted => {
                if let Some(f) = &self.on_delete {
                    f(self.item.clone());
                }
            }
            Msg::ClearAction => {
                self.action.set(None);
            }
        }
    }

    fn render(&mut self, ctx: Context<'_, Self>) -> TagBuilder {
        let registry = context::registry();
        let ty_ident = self.item.data.get_type();
        let entity = ty_ident
            .as_ref()
            .and_then(|ty| registry.entity_by_ident(ty))
            .cloned();
        let ty = entity.as_ref().map(|e| &e.schema.ident);
        let content_renderer = ty
            .as_ref()
            .and_then(|ty| registry.entity_content_renderer(ty));
        let type_name = super::entity_type_name(&self.item.data, entity.as_ref()).map(DomStr::from);

        let mut actions = Vec::new();

        if self.item.data.has_attr::<semantic_core::base::AttrUrl>() {
            actions.push(EntityActionButton {
                // TODO: want to use fas, not fa!
                icon: "fa-globe".into(),
                label: "Go to URL".into(),
                is_active: None,
                is_disabled: false,
                on: Box::new(ctx.callback_msg(|| Msg::Open)),
            })
        }

        actions.push(EntityActionButton {
            // TODO: want to use fas, not fa!
            icon: "fa-table".into(),
            label: "Show Table".into(),
            is_active: Some(Box::pin(self.show_table.signal_cloned())),
            is_disabled: content_renderer.is_none(),
            on: Box::new(ctx.callback_msg(|| Msg::ToggleShowTable)),
        });

        actions.push(EntityActionButton {
            // TODO: want to use fas, not fa!
            icon: "fa-list".into(),
            label: "Manage Collections".into(),
            is_active: None,
            is_disabled: false,
            on: Box::new(ctx.callback_msg(|| Msg::ToggleCollectionManager)),
        });

        actions.push(EntityActionButton {
            // TODO: want to use fas, not fa!
            icon: "fa-tags".into(),
            label: "Manage Tags".into(),
            is_active: None,
            is_disabled: false,
            on: Box::new(ctx.callback_msg(|| Msg::ToggleTagManager)),
        });

        if self.options.editable {
            let is_deleting = self.is_deleting();
            actions.push(EntityActionButton {
                // TODO: want to use fas, not fa!
                icon: "fa-trash".into(),
                label: "Delete".into(),
                is_active: None,
                is_disabled: is_deleting,
                on: Box::new(ctx.callback_msg(|| Msg::DeleteStart)),
            });
        }

        let item = self.item.clone();
        let handle = ctx.handle();
        let active_action_signal = self.action.signal_ref(move |action| {
            match &*action {
                Some(Action::Delete) => {
                    EntityDeleter {
                        item: item.clone(),
                        on_delete: Box::new(handle.callback(|| Msg::Deleted)),
                        on_cancel: Box::new(handle.callback(|| Msg::ClearAction)),
                    }
                    .render()
                }
                Some(Action::ManageCollections) => {
                    if let Some(id) = item.data.get_id() {
                        let content = entity_collection_manager(id);
                        modal(content, handle.callback(|| Msg::ToggleTagManager), true)
                    } else {
                        div()
                    }
                }
                Some(Action::ManageTags) => {
                    if let Some(_id) = item.data.get_id() {
                        let content = entity_tag_manager(&item);
                        modal(content, handle.callback(|| Msg::ToggleTagManager), true)
                    } else {
                        div()
                    }
                }
                None => div(),
            }
        });

        let mut content = div().child_signal(active_action_signal);

        match content_renderer {
            Some(renderer) => {
                tracing::trace!("Using content renderer");
                let item = self.item.clone();
                let renderer = renderer.clone();
                let opts = self.options.clone();
                let registry = registry.clone();
                let signal = self.show_table.signal_cloned().map(move |table| {
                    if table {
                        super::entity_fields_table(&item.data, entity.as_ref(), &registry)
                    } else {
                        renderer(&item, &opts)
                    }
                });
                content.add_child_signal(signal);
            }
            None => {
                content.add_child(super::entity_fields_table(
                    &self.item.data,
                    entity.as_ref(),
                    &registry,
                ));
            }
        };

        // let joins = entity_joins(&self.item, &self.options, registry);

        EntityView {
            title: super::entity_title(&self.item.data).into(),
            type_name,
            on_open: Some(Box::new(ctx.callback_msg(|| Msg::Open))),
            actions,
            content,
        }
        .render()
    }
}
