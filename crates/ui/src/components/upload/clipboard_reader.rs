// Clipboard API resources:
// * https://web.dev/async-clipboard/: has examples and explanations

use brass::{vdom, Callback};
use tracing::trace;

pub struct ClipboardReader {
    pub on_paste: Callback<Vec<web_sys::File>>,
}

enum Msg {
    Paste(web_sys::ClipboardEvent),
}

struct State {
    callback: Callback<Msg>,
    paste_subscription: Option<brass::util::EventSubscription>,
}

brass::enable_props!(wrapped ClipboardReader => State);

impl brass::PropComponent for State {
    type Properties = ClipboardReader;
    type Msg = Msg;

    fn init(_props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            callback: ctx.callback(),
            paste_subscription: None,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Paste(ev) => {
                trace!("files pasted");
                if let Some(files) = ev.clipboard_data().and_then(|d| d.files()) {
                    trace!("got files {}", files.length());
                    let items: Vec<web_sys::File> = (0..files.length())
                        .filter_map(|index| files.item(index))
                        .collect();
                    if !items.is_empty() {
                        props.on_paste.send(items);
                    }
                }
            }
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        _ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        brass_bulma::box_()
            .and(vdom::p_with("Paste files from the clipboard with CTRL-V"))
            .build()
    }

    fn on_render(&mut self, _props: &Self::Properties, first_render: bool) {
        if first_render {
            trace!("subscribing to paste");
            self.paste_subscription = Some(brass::util::EventSubscription::subscribe(
                brass::util::window().into(),
                brass::dom::Event::Paste,
                self.callback.clone().map(Msg::Paste),
            ));
        }
    }
}
