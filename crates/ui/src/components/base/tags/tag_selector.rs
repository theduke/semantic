
pub struct TagSelector {

}

enum Msg {

}

struct State {

}

brass::enable_props!(wrapped TagSelector => State);

impl brass::PropComponent for State {
    type Properties = TagSelector;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self{

        }
    }

    fn update(&mut self, msg: Self::Msg, props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) {
        match msg {

        }
    }

    fn render(&self, props: &Self::Properties, ctx: brass::RenderContext<brass::PropWrapper<Self>>) -> brass::VNode {
        todo!()
    }
}
