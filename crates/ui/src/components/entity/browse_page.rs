use brass::{
    component::{msg::MsgComponent, Context},
    dom::{
        builder::{div, span},
        Render, TagBuilder,
    },
    effect::EffectGuard,
    signal::{
        signal::{Mutable, SignalExt},
        signal_vec::MutableVec,
    },
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, ItemPage, Select},
    },
    schema::{AttrMapExt, EntityDescriptor},
    AnyError,
};

use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_filter::entity_filter},
        loader::Loader,
        util::{box_, buttons, notification_warning, title_2, ButtonBuilder},
    },
    context, EntityRenderOpts,
};

// use super::entity_filter::EntityFilter;

pub struct BrowsePage {
    loader: Loader<()>,
    query: Select,
    _guard: Option<EffectGuard>,
    // filter_callback: Callback<EntityFilter>,
    // on_delete_callback: Callback<Item>,
    items: MutableVec<Item>,
    item_count: Mutable<usize>,

    // TODO: make page size configurable
    limit: u64,
    page: Mutable<u64>,
}

pub struct BrowsePageProps {}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
    FilterUpdated(Expr),
    Next,
    Prev,
    ItemDeleted(Item),
}

impl BrowsePage {
    fn base_filter() -> Expr {
        Expr::not(Expr::in_(
            Expr::attr::<factordb::schema::builtin::AttrType>(),
            vec![semantic_core::base::Tag::QUALIFIED_NAME],
        ))
    }

    fn load(&mut self, query: Select, ctx: &Context<Self>) {
        if self.loader.is_loading() {
            // TODO: queue? abort old?
            return;
        }
        let query2 = query.clone();
        let api = context::api();
        let f = async move { api.select(query2).await };

        let guard = ctx.spawn_map(f, Msg::Loaded);
        self.loader.set_loading(guard);
        self.query = query;
    }
}

impl MsgComponent for BrowsePage {
    type Properties = BrowsePageProps;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: Context<Self>) -> Self {
        let limit = 50;
        let mut s = Self {
            loader: Loader::new_idle(),
            query: Select::new()
                .with_filter(Self::base_filter())
                .with_limit(limit),
            _guard: None,
            items: MutableVec::new(),
            item_count: Mutable::new(0),
            page: Mutable::new(1),
            limit,
            // filter_callback: ctx.callback_map(Msg::FilterUpdated),
            // on_delete_callback: ctx.callback_map(Msg::ItemDeleted),
        };
        s.load(s.query.clone(), &ctx);
        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FilterUpdated(filter) => {
                let query = Select::new().with_filter(filter).with_limit(self.limit);
                self.load(query, &ctx);
            }
            Msg::Loaded(res) => {
                if let Some(page) = self.loader.set_result_take(res) {
                    self.item_count.set(page.items.len());
                    self.items.lock_mut().replace_cloned(page.items);
                }
            }
            Msg::Next => {
                let page = self.page.get();
                self.page.set(page + 1);
                let q = Select {
                    offset: self.limit * page,
                    ..self.query.clone()
                };
                self.load(q, &ctx);
            }
            Msg::Prev => {
                let page = self.page.get();
                if page > 1 {
                    let new_page = page - 1;
                    self.page.set(new_page);
                    let q = Select {
                        offset: self.limit * (new_page - 1),
                        ..self.query.clone()
                    };
                    self.load(q, &ctx);
                }
            }
            Msg::ItemDeleted(deleted_item) => {
                self.items
                    .lock_mut()
                    .retain(|item| item.data.get_id() != deleted_item.data.get_id());
            }
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let loader = self.loader.signal_render(move |_| span());

        let empty_marker = self.item_count.signal().map(|item_count| {
            if item_count == 0 {
                Some(notification_warning().and("Nothing found..."))
            } else {
                None
            }
        });

        let handle = ctx.handle();
        // TODO: do not clone...
        let page_mut = self.page.clone();
        // TODO: make reactive?
        let limit = self.limit as usize;
        let pager = self.item_count.signal().map(move |item_count| {
            let page = page_mut.get();

            let next = if item_count >= limit {
                Some(
                    ButtonBuilder::new()
                        .size_large()
                        .label("Next")
                        .on(handle.callback(|| Msg::Next))
                        .build(),
                )
            } else {
                None
            };

            let prev = if page > 1 {
                Some(
                    ButtonBuilder::new()
                        .size_large()
                        .label("Back")
                        .on(handle.callback(|| Msg::Prev))
                        .build(),
                )
            } else {
                None
            };

            buttons()
                .style_raw("display: flex; justify-content: center;")
                .and(prev)
                .and(next)
        });

        let handle = ctx.handle();
        let handle2 = handle.clone();
        div()
            .and(title_2().and("Browse"))
            .and(box_().and(entity_filter(move |query| {
                handle.send(Msg::FilterUpdated(query.build_expr()));
            })))
            .child_signal(loader)
            .children_signal(self.items.signal_vec_cloned(), move |item| {
                EntityBox {
                    item: item.clone(),
                    options: EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                    on_delete: Some(Box::new(handle2.on(Msg::ItemDeleted))),
                }
                .render()
                .build()
            })
            .child_signal_opt(empty_marker)
            .child_signal(pager)
    }
}

impl brass::dom::Apply for BrowsePageProps {
    fn apply(self, tag: &mut TagBuilder) {
        tag.add_component::<BrowsePage>(self)
    }
}

// fn render_page(
//     items: impl SignalVec<Item = Item> + Unpin + 'static,
//     cursor: impl Signal<Item = Option<Id>> + Unpin + 'static,
//     handle: Handle<BrowsePage>,
// ) -> TagBuilder {

//     div()
//         .children_signal(items, |_item| div().and("Item").build())
//         .child_signal(next)

//     // let opts = EntityRenderOpts {
//     //     editable: false,
//     //     preview: true,
//     // };
//     // let items = page.items.iter().map(|item| {
//     //     // super::entity_view::EntityBox {
//     //     //     item: item.clone(),
//     //     //     options: opts.clone(),
//     //     //     on_delete: Some(cb.clone().map(Msg::ItemDeleted)),
//     //     // };
//     //     div().and("Entity")
//     // });
//     // let next = if page.next_cursor.is_some() {
//     //     // let btn = brass_bulma::button_medium()
//     //     let btn = button().and("More").on(ctx.on(|_: ClickEvent| Msg::Next));
//     //     div().and(btn).build()
//     // } else {
//     //     VNode::Empty
//     // };
// }
