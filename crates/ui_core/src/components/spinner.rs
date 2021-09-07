use brass::EffectGuard;

pub struct DelayedSpinner {}

struct State {
    guard: EffectGuard,
    visible: bool,
}

enum Msg {
    DelayReached,
}

brass::enable_props!(wrapped DelayedSpinner => State);

impl brass::PropComponent for State {
    type Properties = DelayedSpinner;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let guard = ctx.timeout(Msg::DelayReached, std::time::Duration::from_millis(500));
        Self {
            guard,
            visible: false,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::DelayReached => {
                self.visible = true;
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        crate::loader::spinner().build()
    }
}
