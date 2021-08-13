use brass::vdom;
use factordb::{
    query::{
        expr::Expr,
        select::{Item, Page},
    },
    AnyError,
};
use semantic_ui_core::loader::LoadState;

use crate::components::entity::entity_filter::EntityFilterForm;

pub struct Player {}

enum Msg {
    FilterSubmit(Expr),
    PageLoaded(Result<Page<Item>, AnyError>),
    Next,
    Prev,
}

type Index = usize;

struct State {
    loader: LoadState<()>,
    page: Page<Item>,
    cycle: bool,
    paused: bool,
    index: Index,
}

impl State {
    fn next(&mut self, ctx: &mut brass::Context<Msg>) {
        let index = self.index + 1;
        if index + 1 > self.page.items.len() {
            if self.cycle {
                self.goto(0, ctx)
            } else {
                self.paused = true;
            }
        } else {
            self.goto(index, ctx)
        }
    }

    fn prev(&mut self, ctx: &mut brass::Context<Msg>) {
        if self.index == 0 {
            if self.cycle {}
        } else {
            self.goto(self.index - 1, ctx)
        }
    }

    fn goto(&mut self, index: Index, ctx: &mut brass::Context<Msg>) {
        debug_assert!(index + 1 < self.page.items.len());
        self.index = index;
    }
}

impl brass::PropComponent for State {
    type Properties = Player;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            loader: LoadState::Idle,
            page: Page::new(),
            cycle: true,
            paused: false,
            index: 0,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::FilterSubmit(expr) => {}
            Msg::PageLoaded(res) => match res {
                Ok(page) => {
                    let is_first = self.page.items.is_empty();

                    self.page.items.extend(page.items);
                    self.page.next_cursor = page.next_cursor;
                    self.loader.set_success(());

                    if !self.paused && is_first {
                        self.next(ctx);
                    }
                }
                Err(err) => {
                    self.loader.set_failed(err);
                }
            },
            Msg::Next => {}
            Msg::Prev => {}
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        mut ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let filter = EntityFilterForm {
            on_submit: ctx.callback_map(Msg::FilterSubmit),
        };

        vdom::div().and(filter).build()
    }
}
