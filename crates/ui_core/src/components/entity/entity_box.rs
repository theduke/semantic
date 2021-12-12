use brass::{
    component::{msg::MsgComponent, Context},
    dom::{builder::div, Render, TagBuilder, View},
    signal::signal::{Mutable, SignalExt},
    DomStr,
};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::entity_title;
use web_sys::Element;

use crate::{
    base::{collection::entity_collection_manager, tags::entity_tag_manager},
    components::util::modal::modal,
    context,
    routing::Route,
    EntityRenderOpts, SharedRenderer0,
};

use super::{
    entity_deleter::EntityDeleter,
    entity_view::{EntityActionButton, EntityView},
};

pub struct EntityBox {
    pub item: Item,
    pub options: EntityRenderOpts,
    pub on_delete: Option<Box<dyn Fn(Item)>>,
    pub show_link: bool,
}

impl Render for EntityBox {
    fn render(self) -> View {
        brass::component::build_component::<State>(self)
    }
}

pub struct Action {
    pub icon: DomStr<'static>,
    pub label: DomStr<'static>,
    pub render: SharedRenderer0,
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

#[derive(Clone, Debug)]
enum ActiveAction {
    Delete,
    ManageCollections,
    ManageTags,
}

struct State {
    item: Mutable<Item>,
    // entity_id: Option<Id>,
    options: EntityRenderOpts,
    on_delete: Option<Box<dyn Fn(Item)>>,
    show_table: Mutable<bool>,
    action: Mutable<Option<ActiveAction>>,

    root_div: Option<Element>,
}

impl MsgComponent for State {
    type Properties = EntityBox;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: Context<'_, Self>) -> Self {
        Self {
            item: Mutable::new(props.item),
            on_delete: props.on_delete,
            options: props.options,
            show_table: Mutable::new(false),
            action: Mutable::new(None),
            root_div: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: Context<'_, Self>) {
        match msg {
            Msg::ToggleActions => {
                todo!()
            }
            Msg::Open => {
                if let Some(id) = self.item.lock_ref().data.get_id() {
                    context::router().goto(Route::Entity(id.into()));
                }
            }
            Msg::OpenSourceUrl => {
                if let Some(url) = self
                    .item
                    .lock_ref()
                    .data
                    .get_attr::<semantic_core::base::AttrUrl>()
                {
                    let _ = brass::web::window()
                        .open_with_url_and_target(url.as_str(), "_blank")
                        .map_err(|_err| {
                            tracing::error!("Could not open window");
                        });
                }
            }
            Msg::DeleteStart => {
                self.action.set(Some(ActiveAction::Delete));
            }
            Msg::ToggleShowTable => {
                self.show_table.replace_with(|old| !*old);
            }
            Msg::ToggleCollectionManager => {
                self.action.replace_with(|old| {
                    if matches!(old, Some(ActiveAction::ManageCollections)) {
                        None
                    } else {
                        Some(ActiveAction::ManageCollections)
                    }
                });
            }
            Msg::ToggleTagManager => {
                self.action.replace_with(|old| {
                    if matches!(old, Some(ActiveAction::ManageTags)) {
                        None
                    } else {
                        Some(ActiveAction::ManageTags)
                    }
                });
            }
            Msg::Deleted => {
                if let Some(f) = &self.on_delete {
                    f(self.item.lock_ref().clone());
                } else if let Some(root) = self.root_div.take() {
                    // If no on_delete callback is given, just remove the node
                    // from the dom.
                    // TODO: use an animation!
                    root.parent_element()
                        .map(|parent| parent.remove_child(&root).ok());
                }
            }
            Msg::ClearAction => {
                self.action.set(None);
            }
        }
    }

    fn render(&mut self, ctx: Context<'_, Self>) -> TagBuilder {
        let mutable_item = self.item.clone();

        let handle = ctx.handle();
        let options = self.options.clone();
        let show_table = self.show_table.clone();
        let action = self.action.clone();

        let content_signal = self.item.signal_ref(move |item| {
            let registry = context::registry();
            let ty_ident = item.data.get_type();
            let entity = ty_ident
                .as_ref()
                .and_then(|ty| registry.entity_by_ident(ty))
                .cloned();
            let ty = entity.as_ref().map(|e| &e.schema.ident);
            let content_renderer = ty
                .as_ref()
                .and_then(|ty| registry.entity_content_renderer(ty));
            let type_name = super::entity_type_name(&item.data, entity.as_ref()).map(DomStr::from);

            let mut actions = Vec::new();

            if item.data.has_attr::<semantic_core::base::AttrUrl>() {
                actions.push(EntityActionButton {
                    // TODO: want to use fas, not fa!
                    icon: "fa-globe".into(),
                    label: "Go to URL".into(),
                    is_active: None,
                    is_disabled: false,
                    on: Box::new(handle.callback(|| Msg::Open)),
                })
            }

            actions.push(EntityActionButton {
                // TODO: want to use fas, not fa!
                icon: "fa-table".into(),
                label: "Show Table".into(),
                is_active: Some(Box::pin(show_table.signal_cloned())),
                is_disabled: content_renderer.is_none(),
                on: Box::new(handle.callback(|| Msg::ToggleShowTable)),
            });

            actions.push(EntityActionButton {
                // TODO: want to use fas, not fa!
                icon: "fa-list".into(),
                label: "Manage Collections".into(),
                is_active: None,
                is_disabled: false,
                on: Box::new(handle.callback(|| Msg::ToggleCollectionManager)),
            });

            actions.push(EntityActionButton {
                // TODO: want to use fas, not fa!
                icon: "fa-tags".into(),
                label: "Manage Tags".into(),
                is_active: None,
                is_disabled: false,
                on: Box::new(handle.callback(|| Msg::ToggleTagManager)),
            });

            if options.editable {
                // let is_deleting = self.is_deleting();
                actions.push(EntityActionButton {
                    // TODO: want to use fas, not fa!
                    icon: "fa-trash".into(),
                    label: "Delete".into(),
                    is_active: None,
                    is_disabled: false,
                    on: Box::new(handle.callback(|| Msg::DeleteStart)),
                });
            }

            // let mutable_item = self.item.clone();

            let id = item.data.get_id();

            let registry = registry.clone();
            let mutable_item = mutable_item.clone();

            let handle2 = handle.clone();
            let active_action_signal = action.signal_ref(move |action| -> View {
                if let Some(action) = action {
                    let handle = handle2.clone();
                    let content = match action {
                        ActiveAction::Delete => EntityDeleter {
                            item: mutable_item.lock_ref().clone(),
                            on_delete: Box::new(handle.callback(|| Msg::Deleted)),
                            on_cancel: Box::new(handle.callback(|| Msg::ClearAction)),
                        }
                        .render(),
                        ActiveAction::ManageCollections => {
                            if let Some(id) = &id {
                                let content = entity_collection_manager(*id);
                                modal(content, handle.callback(|| Msg::ClearAction), true).into()
                            } else {
                                div().into()
                            }
                        }
                        ActiveAction::ManageTags => {
                            if let Some(_id) = &id {
                                let content = entity_tag_manager(&mutable_item.lock_ref().clone());
                                modal(content, handle.callback(|| Msg::ClearAction), true).into()
                            } else {
                                div().into()
                            }
                        }
                    };
                    content
                } else {
                    View::Empty
                }
            });

            let mut content = div().signal(active_action_signal);

            match content_renderer {
                Some(renderer) => {
                    let item = item.clone();
                    let renderer = renderer.clone();
                    let opts = options.clone();
                    let registry = registry.clone();
                    let signal = show_table.signal_cloned().map(move |table| {
                        if table {
                            super::entity_fields_table(&item.data, entity.as_ref(), &registry)
                        } else {
                            renderer(&item, &opts)
                        }
                    });
                    content.add_signal(signal);
                }
                None => {
                    content.add_child(super::entity_fields_table(
                        &item.data,
                        entity.as_ref(),
                        &registry,
                    ));
                }
            };

            EntityView {
                link_path: super::entity_href(item),
                title: entity_title(&item.data).into(),
                type_name,
                on_open: Some(Box::new(handle.callback(|| Msg::Open))),
                actions,
                content,
            }
            .render()
        });

        let root = div().signal(content_signal);

        self.root_div = Some(root.elem().clone());

        root
    }
}
