use brass::{
    component::{msg::MsgComponent, Context, Handle},
    dom::{
        builder::{button, div, span},
        ClickEvent, Render, TagBuilder,
    },
    effect::EffectGuard,
    signal::{
        signal::{Mutable, MutableSignal, Signal, SignalExt},
        signal_vec::{MutableSignalVec, MutableVec, SignalVec, SignalVecExt},
    },
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, ItemPage, Select},
    },
    schema::{AttrMapExt, EntityDescriptor},
    AnyError, Id,
};

use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_filter::entity_filter},
        loader::{LoadState, Loader},
        util::{box_, notification_warning, title_2},
    },
    context, EntityRenderOpts,
};

// use super::entity_filter::EntityFilter;

pub struct BrowsePage {
    loader: Loader<()>,
    query: Select,
    guard: Option<EffectGuard>,
    // filter_callback: Callback<EntityFilter>,
    // on_delete_callback: Callback<Item>,
    items: MutableVec<Item>,
    is_empty: Mutable<bool>,
    next_cursor: Mutable<Option<Id>>,
}

pub struct BrowsePageProps {}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
    FilterUpdated(Expr),
    Next,
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
        let mut s = Self {
            loader: Loader::new_idle(),
            query: Select::new().with_filter(Self::base_filter()),
            guard: None,
            items: MutableVec::new(),
            is_empty: Mutable::new(true),
            next_cursor: Mutable::new(None),
            // filter_callback: ctx.callback_map(Msg::FilterUpdated),
            // on_delete_callback: ctx.callback_map(Msg::ItemDeleted),
        };
        s.load(s.query.clone(), &ctx);
        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FilterUpdated(filter) => {
                let query = Select::new().with_filter(filter);
                self.load(query, &ctx);
            }
            Msg::Loaded(res) => {
                if let Some(page) = self.loader.set_result_take(res) {
                    self.is_empty.set(page.items.is_empty());
                    self.items.lock_mut().replace_cloned(page.items);
                    self.next_cursor.set(page.next_cursor)
                }
            }
            Msg::Next => {
                if let Some(cursor) = self.next_cursor.get() {
                    let q = Select {
                        cursor: Some(cursor),
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

        let empty_marker = self.is_empty.signal().map(|is_empty| {
            if is_empty {
                Some(notification_warning().and("Nothing found..."))
            } else {
                None
            }
        });

        let handle = ctx.handle();
        let next = self.next_cursor.signal().map(move |cursor| {
            if cursor.is_some() {
                Some(
                    div().and(
                        button()
                            .and("More")
                            .on(handle.on(|_: ClickEvent| Msg::Next)),
                    ),
                )
            } else {
                None
            }
        });

        let handle = ctx.handle();
        div()
            .and(title_2().and("Browse"))
            .and(box_().and(entity_filter(move |query| {
                handle.send(Msg::FilterUpdated(query));
            })))
            .child_signal(loader)
            .children_signal(self.items.signal_vec_cloned(), |item| {
                EntityBox {
                    item: item.clone(),
                    options: EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                    on_delete: None,
                }
                .render()
                .build()
            })
            .child_signal_opt(empty_marker)
            .child_signal_opt(next)
        // .and((filter, loader)).build()
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
