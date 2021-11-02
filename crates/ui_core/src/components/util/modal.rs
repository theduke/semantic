use std::rc::Rc;

use brass::{
    dom::{builder::div, Apply, Attr, ClickEvent, TagBuilder},
    effect::EventSubscription,
    web::window,
};

use super::Cls;

pub fn modal<C: Apply>(
    content: C,
    on_close: impl Fn() + 'static,
    handle_escape: bool,
) -> TagBuilder {
    let on_close = Rc::new(on_close);

    let on_close2 = on_close.clone();
    let bg = div()
        .class("modal-background")
        .on(move |_: ClickEvent| on_close2());

    let inner = div().class(Cls::ModalContent).and(content);

    let on_close2 = on_close.clone();
    let close = super::button()
        .class(Cls::ModalClose)
        .class(super::Size::Large)
        .attr(Attr::AriaLabel, "Close")
        .on(move |_: ClickEvent| on_close2());

    let mut root = div()
        .class(Cls::Modal)
        .class(Cls::IsActive)
        .and(bg)
        .and(inner)
        .and(close);

    if handle_escape {
        let on_close = on_close.clone();
        let sub = EventSubscription::subscribe(
            window().clone().into(),
            brass::dom::Event::KeyDown,
            move |ev: web_sys::KeyboardEvent| {
                if ev.code() == "Escape" {
                    on_close()
                }
            },
        );
        root.add_bind(sub);
    };

    root
}
