pub mod form;
mod loader;
pub mod markdown;

use brass::{vdom, VNode};
pub use loader::{Loader, LoaderFunc};

pub fn small_title(content: impl Into<String>) -> VNode {
    vdom::div()
        .class("mb-4")
        .and(vdom::b().and(content.into()))
        .build()
}
