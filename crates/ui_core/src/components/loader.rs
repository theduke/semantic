use std::marker::PhantomData;

use brass::{vdom::Func, PropComponent, Shared};
use factordb::AnyError;
use futures::future::LocalBoxFuture;

use crate::loader::LoadState;

pub type LoaderFunc<I, O> = Func<I, LocalBoxFuture<'static, Result<O, AnyError>>>;

pub struct Loader<I, O> {
    pub input: I,
    pub load: LoaderFunc<I, O>,
    pub render: brass::vdom::Renderer<Shared<O>>,
}

impl<I, O> brass::vdom::Render for Loader<I, O>
where
    I: Eq + Clone + 'static,
    O: Unpin + 'static,
{
    fn render(self) -> brass::VNode {
        State::build(self)
    }
}

enum Msg<O> {
    Loaded(Result<O, AnyError>),
}

struct State<I, O> {
    status: LoadState<Shared<O>>,
    _input: PhantomData<I>,
}

impl<I, O> brass::PropComponent for State<I, O>
where
    I: Eq + Clone + 'static,
    O: Unpin + 'static,
{
    type Properties = Loader<I, O>;
    type Msg = Msg<O>;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let f = props.load.call(props.input.clone());
        let guard = ctx.run_map(f, Msg::Loaded);
        let status = LoadState::Loading(Some(guard));

        Self {
            status,
            _input: PhantomData,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        _props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Loaded(res) => {
                self.status.set_result(res.map(Shared::new));
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        _ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        self.status
            .render(|output| props.render.render(output.clone()))
    }
}
