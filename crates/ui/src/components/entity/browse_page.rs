use brass::{
    component::{msg::MsgComponent, Context, Handle},
    dom::{builder::div, TagBuilder},
    effect::EffectGuard,
    signal::signal::Mutable,
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, ItemPage, Select},
    },
    AnyError,
};

use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_filter::entity_filter},
        loader::Loader,
        util::{box_, buttons, notification_warning, title_2, ButtonBuilder, Color},
    },
    context, EntityRenderOpts,
};

pub struct BrowsePageProps {}

// use super::entity_filter::EntityFilter;

struct LoadedPage {
    items: Vec<Item>,
    page: usize,
    limit: usize,
}

pub struct BrowsePage {
    query: Select,

    loader: Loader<LoadedPage>,
    _guard: Option<EffectGuard>,

    filter_visible: Mutable<bool>,

    // TODO: make page size configurable
    limit: u64,
    page: usize,
}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
    FilterUpdated(Expr),
    ToggleFilter,
    Next,
    Prev,
}

impl BrowsePage {
    fn base_filter() -> Expr {
        let ignored: Vec<_> = context::registry()
            .ignored_entity_types()
            .iter()
            .cloned()
            .collect();

        Expr::not(Expr::in_(
            Expr::attr::<factordb::schema::builtin::AttrType>(),
            ignored,
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
            page: 1,
            limit,
            filter_visible: Mutable::new(false),
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
                let res = res.map(|page| LoadedPage {
                    items: page.items,
                    page: self.page,
                    limit: self.limit as usize,
                });
                self.loader.set_result(res);
            }
            Msg::Next => {
                if self.loader.is_loading() {
                    return;
                }
                let page = self.page;
                self.page += 1;
                let q = Select {
                    offset: self.limit * page as u64,
                    ..self.query.clone()
                };
                self.load(q, &ctx);
            }
            Msg::Prev => {
                if self.loader.is_loading() {
                    return;
                }
                let page = self.page;
                if page > 1 {
                    let new_page = page - 1;
                    self.page = new_page;
                    let q = Select {
                        offset: self.limit * (new_page as u64 - 1),
                        ..self.query.clone()
                    };
                    self.load(q, &ctx);
                }
            }
            Msg::ToggleFilter => {
                self.filter_visible.replace_with(|x| !*x);
                tracing::trace!(visible = self.filter_visible.get(), "filter visible");
            }
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let action_bar = box_().and(
            ButtonBuilder::new()
                .icon("fas fa-search")
                .on(ctx.callback_msg(|| Msg::ToggleFilter))
                .build()
                .class_signal_toggle(Color::Info, self.filter_visible.signal()),
        );

        let handle = ctx.handle();
        let filter = box_()
            .style_signal(
                brass::dom::Style::Display,
                self.filter_visible
                    .signal_ref(|flag| if *flag { "block" } else { "none" }),
            )
            .and(entity_filter(move |query| {
                handle.send(Msg::FilterUpdated(query.build_expr()));
            }));

        let handle = ctx.handle();
        let content = self.loader.signal_render(move |page| {
            if page.items.is_empty() {
                notification_warning().and("Nothing found...").into_view()
            } else {
                let items = page.items.iter().map(|item| EntityBox {
                    item: item.clone(),
                    show_link: true,
                    options: EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                    on_delete: None,
                });

                let pager = render_pager(page.items.len(), page.page, page.limit, &handle);

                div().and_iter(items).and(pager).into_view()
            }
        });

        div()
            .and(title_2().and("Browse"))
            .and(action_bar)
            .and(filter)
            .signal(content)
    }
}

fn render_pager(
    item_count: usize,
    page: usize,
    limit: usize,
    handle: &Handle<BrowsePage>,
) -> TagBuilder {
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
}

impl brass::dom::Apply for BrowsePageProps {
    fn apply(self, tag: &mut TagBuilder) {
        tag.add_component::<BrowsePage>(self)
    }
}
