use brass::vdom::{div, Ref};

pub struct Props<S> {
    pub markdown: S,
}

pub struct Markdown<S> {
    markdown: S,
    html: String,
    vref: Ref,
}

fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};

    let parser = Parser::new_ext(markdown, Options::all());
    let mut output = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut output, parser);
    output
}

impl<S: AsRef<str> + Eq + 'static> brass::Component for Markdown<S> {
    type Properties = Props<S>;
    type Msg = ();

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            html: markdown_to_html(props.markdown.as_ref()),
            markdown: props.markdown,
            vref: Ref::new(),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            () => {}
        }
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        div().class("content").build_ref(&self.vref)
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props.markdown != self.markdown {
            self.html = markdown_to_html(props.markdown.as_ref());
            self.markdown = props.markdown;
            true
        } else {
            false
        }
    }

    fn on_render(&mut self, _first_render: bool) {
        if let Some(elem) = self.vref.get() {
            elem.set_inner_html(&self.html);
        } else {
            tracing::error!("could not obtain reference to markdown div");
        }
    }
}
