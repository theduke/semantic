use dioxus::prelude::*;

use super::Color;

fn alert_class(color: Color) -> &'static str {
    match color {
        Color::Primary => "alert alert-primary",
        Color::Secondary => "alert alert-secondary",
        Color::Success => "alert alert-success",
        Color::Danger => "alert alert-danger",
        Color::Warning => "alert alert-warning",
        Color::Info => "alert alert-info",
        Color::Light => "alert alert-light",
        Color::Dark => "alert alert-dark",
    }
}

#[inline_props]
pub fn Alert<'a>(cx: Scope<'a>, color: Color, children: Element<'a>) -> Element {
    let cls = alert_class(*color);
    cx.render(rsx! {
        div {
            class: cls,
            role: "alert",
            children
        }
    })
}
