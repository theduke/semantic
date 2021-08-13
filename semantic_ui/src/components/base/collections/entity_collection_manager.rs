use std::collections::HashSet;

use brass::vdom;
use factordb::{
    data::value::from_value_map,
    query::{
        expr::Expr,
        select::{Item, Page},
    },
    schema::{builtin::AttrType, EntityDescriptor},
    AnyError, Id,
};
use semantic_ui_core::{
    components::small_title,
    loader::{error_msg, LoadState},
};
use semantics_core::base::Collection;

use crate::components::entity::entity_search_autocomplete::EntitySearchAutocomplete;

/// Allows managing the collections an entity is part of.
pub struct EntityCollectionManager {
    pub entity_id: Id,
}

enum Msg {
    CurrentLoaded(Result<Page<Collection>, AnyError>),
    Remove(Collection),
    RemoveLoaded(Result<Collection, AnyError>),
    AddCollection(Item),
    AddLoaded(Result<Collection, AnyError>),
}

struct State {
    current_collections_loader: LoadState<Page<Collection>>,
    remove_loader: LoadState<()>,
    add_loader: LoadState<()>,
}

brass::enable_props!(wrapped EntityCollectionManager => State);

impl brass::PropComponent for State {
    type Properties = EntityCollectionManager;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let entity_id = props.entity_id;
        let guard = ctx.run_map(
            async move {
                let select = Collection::query_collections_with_entity(entity_id);
                let page = crate::api()
                    .select(select)
                    .await?
                    .convert_data::<Collection>()?;
                Ok(page)
            },
            Msg::CurrentLoaded,
        );

        Self {
            current_collections_loader: LoadState::Loading(Some(guard)),
            remove_loader: LoadState::Idle,
            add_loader: LoadState::Idle,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::CurrentLoaded(res) => {
                self.current_collections_loader.set_result(res);
            }
            Msg::Remove(mut col) => {
                let entity_id = props.entity_id;
                let guard = ctx.run_map(
                    async move {
                        let mutate = Collection::mutate_remove_item(col.clone(), entity_id)?;
                        crate::api().mutate(mutate).await?;
                        Ok(col)
                    },
                    Msg::RemoveLoaded,
                );
                self.remove_loader.set_loading_guarded(guard);
            }
            Msg::RemoveLoaded(res) => match res {
                Ok(col) => {
                    self.current_collections_loader
                        .as_success_mut()
                        .map(|page| page.items.retain(|item| item.id != col.id));
                    self.remove_loader.set_idle();
                }
                Err(err) => {
                    self.remove_loader.set_failed(err);
                }
            },
            Msg::AddCollection(item) => {
                if self.add_loader.is_loading() {
                    return;
                }
                let entity_id = props.entity_id;
                let guard = ctx.run_map(
                    async move {
                        let col: Collection = from_value_map(item.data)?;
                        let mutate = Collection::mutate_add_item(col.id, entity_id);
                        crate::api().mutate(mutate).await?;
                        Ok(col)
                    },
                    Msg::AddLoaded,
                );
                self.add_loader.set_loading_guarded(guard);
            }
            Msg::AddLoaded(res) => match res {
                Ok(col) => {
                    if let Some(page) = self.current_collections_loader.as_success_mut() {
                        page.items.push(col);
                    }
                    self.add_loader.set_success(());
                }
                Err(err) => {
                    self.add_loader.set_failed(err);
                }
            },
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        mut ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        self.current_collections_loader.render(|cols| {
            let remove_loading = self.remove_loader.is_loading();

            let current_collections = if cols.items.is_empty() {
                brass_bulma::notification(brass_bulma::Color::Default, "Not in any collection.")
                    .build()
            } else {
                let items = cols.items.iter().map(|collection| {
                    let title = collection.title.clone();
                    let collection2 = collection.clone();
                    let toggle = vdom::button()
                        .and_class(brass_bulma::Color::Danger.as_class())
                        .and_class("ml-4")
                        .and(brass_bulma::icon_fa("fas fa-minus-circle"))
                        .style_raw("pl-4")
                        .attr_toggle_if(remove_loading, brass::dom::Attr::Disabled)
                        .on_click(ctx.on_simple(move || Msg::Remove(collection2.clone())));
                    vdom::div().class("mb-2").and(title).and(toggle)
                });

                vdom::div().and_iter(items).build()
            };

            let remove_err = self
                .remove_loader
                .as_error()
                .map(|err| error_msg(err))
                .unwrap_or_else(|| vdom::div());

            let mut ignored: HashSet<Id> = self
                .current_collections_loader
                .as_success()
                .map(|page| page.items.iter().map(|x| x.id).collect::<HashSet<_>>())
                .unwrap_or_default();
            ignored.insert(props.entity_id);

            let finder = EntitySearchAutocomplete {
                placeholder: Some("Collection title...".into()),
                filter: Some(Expr::eq(
                    Expr::attr::<AttrType>(),
                    Collection::QUALIFIED_NAME,
                )),
                on_select: ctx.callback_map(Msg::AddCollection),
                ignored_ids: Some(ignored),
            };

            let finder_wrap = vdom::div().and((small_title("Add to Collection"), finder));

            vdom::div()
                .and((
                    small_title("Collections"),
                    current_collections,
                    remove_err,
                    vdom::hr(),
                    finder_wrap,
                ))
                .build()
        })
    }
}
