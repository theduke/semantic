use crate::Viewport;
use dioxus::prelude::*;

#[component]
pub fn GraphBackground(viewport: Signal<Viewport>) -> Element {
    let viewport = viewport();
    rsx! {
        div {
            class: "dxgraph-background",
            aria_hidden: "true",
            style: "--dxg-x:{viewport.x}px;--dxg-y:{viewport.y}px;--dxg-zoom:{viewport.zoom}",
        }
    }
}

/// Only this wrapper subscribes to viewport changes; its children are memoized.
#[component]
pub(crate) fn GraphWorld(viewport: Signal<Viewport>, children: Element) -> Element {
    let viewport = viewport();
    rsx! {
        div {
            class: "dxgraph-world",
            style: "transform:translate({viewport.x}px,{viewport.y}px) scale({viewport.zoom});transform-origin:0 0",
            {children}
        }
    }
}
