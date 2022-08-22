use brass::{
    component::{msg::MsgComponent, Context},
    dom::{builder::div, Attr, ClickEvent, Render, TagBuilder, View},
    signal::{signal::Mutable, signal_vec::MutableVec},
};
use factdb::{AttrId, AttrMapExt, AttributeMeta, DataMap, Expr, Id};
use semantic_core::base::Collection;

use crate::{
    components::{
        autocomplete::entity_picker::entity_picker,
        entity::entity_view::EntityView,
        loader::Loader,
        util::{box_, button, buttons, icon_fas, subtitle_4},
    },
    context::{self, api},
    EntityRenderOpts, Registry,
};

enum AddItemMode {
    None,
    AddExisting,
}

pub struct CollectionItemManager {
    pub collection_id: Id,
    pub items: MutableVec<DataMap>,
}

impl Render for CollectionItemManager {
    fn render(self) -> View {
        brass::component::build_component::<State>(self)
    }
}

struct State {
    collection_id: Id,
    items: MutableVec<DataMap>,
    mode: Mutable<AddItemMode>,
    filter_ignored_ids: Mutable<Vec<Id>>,
    loader: Loader<()>,
}

enum Msg {
    Remove(Id),
    RemoveLoaded(Id),
    Add(DataMap),
    AddLoaded(DataMap),

    ModeAddExisting,
}

impl MsgComponent for State {
    type Properties = CollectionItemManager;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: Context<Self>) -> Self {
        Self {
            collection_id: props.collection_id,
            items: props.items,
            mode: Mutable::new(AddItemMode::None),
            loader: Loader::new_idle(),
            filter_ignored_ids: Mutable::new(vec![props.collection_id]),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::Remove(id) => {
                if self.loader.is_loading() {
                    return;
                }

                let collection_id = self.collection_id;

                let handle = ctx.handle();
                self.loader.spawn(async move {
                    api()
                        .mutate(Collection::mutate_remove_item(collection_id, id))
                        .await?;

                    handle.send(Msg::RemoveLoaded(id));

                    Ok(())
                });
            }
            Msg::RemoveLoaded(id) => {
                self.items
                    .lock_mut()
                    .retain(|item| item.get_id() != Some(id));
                // Remove id from filter.
                self.filter_ignored_ids.lock_mut().retain(|x| *x != id);
            }
            Msg::Add(item) => {
                if self.loader.is_loading() {
                    return;
                }

                let collection_id = self.collection_id;
                let item_id = if let Some(id) = item.get_id() {
                    id
                } else {
                    return;
                };

                let handle = ctx.handle();
                self.loader.spawn(async move {
                    api()
                        .mutate(Collection::mutate_add_item(collection_id, item_id))
                        .await?;

                    handle.send(Msg::AddLoaded(item));

                    Ok(())
                });
            }
            Msg::AddLoaded(item) => {
                if let Some(id) = item.get_id() {
                    self.filter_ignored_ids.lock_mut().push(id);
                }
                self.items.lock_mut().push_cloned(item);
            }
            Msg::ModeAddExisting => {
                self.mode.set(AddItemMode::AddExisting);
            }
        }
    }

    fn render(&mut self, ctx: Context<'_, Self>) -> brass::dom::TagBuilder {
        let handle = ctx.handle();
        let registry = context::registry();

        let error = self.loader.signal_render(|_| View::Empty);

        let items = div().signal_vec(self.items.signal_vec_cloned(), move |item| {
            let on_remove = handle.on(Msg::Remove);
            render_item(&item, &registry, on_remove).build()
        });

        let handle = ctx.handle();
        let filter_ignored_ids = self.filter_ignored_ids.clone();

        let adder = self.mode.signal_ref(move |mode| match mode {
            AddItemMode::None => {
                let btn_add = button()
                    .and(icon_fas("fa-search-plus"))
                    .and("Add")
                    .attr(Attr::Title, "Add existing entity")
                    .on(handle.on(|_: ClickEvent| Msg::ModeAddExisting));

                buttons().and(btn_add)
            }
            AddItemMode::AddExisting => {
                let ignored = filter_ignored_ids
                    .signal_ref(|ids| Expr::not(Expr::in_(AttrId::expr(), ids.clone())));
                let picker = entity_picker(ignored, handle.on(Msg::Add), None);

                box_().and(subtitle_4().and("Find")).and(picker)
            }
        });

        div().signal(error).and(items).signal(adder)
    }
}

fn render_item(
    item: &DataMap,
    registry: &Registry,
    on_remove: impl Fn(Id) + 'static,
) -> TagBuilder {
    let opts = EntityRenderOpts {
        editable: false,
        preview: true,
    };

    let view = EntityView::from_item(item, registry, &opts);
    let view_wrap = div().class("is-flex-grow-1").and(view);

    let id = item.get_id().unwrap_or(Id::nil());
    let btn_remove = button()
        .and(icon_fas("fa-minus-circle"))
        .attr(Attr::Title, "Remove")
        .on(move |_: ClickEvent| {
            on_remove(id);
        });

    let actions = div().class("ml-4").and(btn_remove);

    div().class("is-flex").and((view_wrap, actions))
}
