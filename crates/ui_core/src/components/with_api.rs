use brass::vdom::RefRenderer;

use crate::RenderContextExt;

pub struct WithApi {
    pub render: RefRenderer<crate::api::BrowserApiClient>,
}

struct State;

brass::enable_props!(wrapped WithApi => State);

impl brass::PropComponent for State {
    type Properties = WithApi;
    type Msg = ();

    fn init(_props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self
    }

    fn update(
        &mut self,
        _msg: Self::Msg,
        _props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        props.render.render(ctx.api())
    }
}
