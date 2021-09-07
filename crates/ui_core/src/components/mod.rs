pub mod form;
mod loader;
pub mod markdown;

pub mod autocomplete;

mod spinner;
pub use self::spinner::DelayedSpinner;

mod with_api;
pub use with_api::WithApi;

mod with_registry;
pub use with_registry::WithRegistry;

use brass::{vdom, VNode};
pub use loader::{Loader, LoaderFunc, LoaderFuture};

pub fn small_title(content: impl Into<String>) -> VNode {
    vdom::div()
        .class("mb-4")
        .and(vdom::b().and(content.into()))
        .build()
}
