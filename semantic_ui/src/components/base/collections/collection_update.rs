use anyhow::Result;
use brass::{vdom::Render, Shared};
use factordb::{query::select::Item, schema::EntityContainer, AnyError};
use semantic_core::base::{Collection, CollectionWithItems};
use semantic_ui_core::loader::LoadState;

use super::collection_form::CollectionForm;

pub struct CollectionUpdate {
    pub item: Item,
}

enum Msg {
    Loaded(Result<Shared<CollectionWithItems>, AnyError>),
    Submit(Collection),
    SubmitLoaded(Result<(), AnyError>),
}

struct State {
    init_loader: LoadState<Shared<CollectionWithItems>>,
    persist_loader: LoadState<()>,
}

brass::enable_props!(wrapped CollectionUpdate => State);

impl brass::PropComponent for State {
    type Properties = CollectionUpdate;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let loader =
            match factordb::data::value::from_value_map::<_, Collection>(props.item.data.clone()) {
                Ok(col) => {
                    let guard = ctx.run_map(
                        async move {
                            let query = CollectionWithItems::build_query(&col);
                            let page = crate::api().select(query).await?;
                            let col = CollectionWithItems {
                                collection: col,
                                items: page.items,
                            };
                            Ok(Shared::new(col))
                        },
                        Msg::Loaded,
                    );
                    LoadState::Loading(Some(guard))
                }
                Err(err) => LoadState::Failed(err.to_string()),
            };

        Self {
            init_loader: loader,
            persist_loader: LoadState::Idle,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        _props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Loaded(res) => {
                self.init_loader.set_result(res);
            }
            Msg::Submit(col) => {
                if let Ok(data) = col.into_map() {
                    let guard = ctx.run_map(
                        async move {
                            crate::api()
                                .mutate(
                                    factordb::query::mutate::Mutate::merge_from_map(data).unwrap(),
                                )
                                .await
                        },
                        Msg::SubmitLoaded,
                    );
                    self.persist_loader.set_loading_guarded(guard);
                }
            }
            Msg::SubmitLoaded(res) => {
                self.persist_loader.set_result(res);
            }
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        mut ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        self.init_loader.render(|col| {
            CollectionForm {
                auto_edit_metadata: false,
                item: col.clone(),
                on_submit: ctx.callback_map(Msg::Submit),
                is_loading: self.persist_loader.is_loading(),
                error: self.persist_loader.as_error().map(|x| x.to_string()),
            }
            .render()
        })
    }
}
