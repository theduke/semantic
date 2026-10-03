use crate::{NodeId, Point, Rect, Size, Viewport, ViewportLimits};
use dioxus::prelude::*;
use indexmap::IndexMap;

const FIT_PADDING: f64 = 40.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ControllerGeometry {
    pub revision: u64,
    pub container: Size,
    pub rects: IndexMap<NodeId, Rect>,
    pub limits: ViewportLimits,
}
impl Default for ControllerGeometry {
    fn default() -> Self {
        Self {
            revision: 0,
            container: Size {
                width: 800.0,
                height: 600.0,
            },
            rects: IndexMap::new(),
            limits: ViewportLimits::default(),
        }
    }
}

/// A canvas handle. Callers own durable positions; this handle owns the viewport.
#[derive(Clone, Copy, PartialEq)]
pub struct GraphController {
    pub(crate) viewport: Signal<Viewport>,
    pub(crate) geometry: Signal<ControllerGeometry>,
    pub(crate) revision: Signal<u64>,
}
pub fn use_graph_controller() -> GraphController {
    GraphController {
        viewport: use_signal(Viewport::default),
        geometry: use_signal(ControllerGeometry::default),
        revision: use_signal(|| 0),
    }
}
impl GraphController {
    pub fn viewport(self) -> Viewport {
        *self.viewport.peek()
    }
    pub fn set_viewport(mut self, viewport: Viewport) {
        let limits = self.geometry.peek().limits;
        self.viewport.set(Viewport {
            zoom: viewport.zoom.clamp(limits.min_zoom, limits.max_zoom),
            ..viewport
        });
    }
    pub fn zoom_by(mut self, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let geometry = self.geometry.peek();
        let mut viewport = *self.viewport.peek();
        viewport = viewport.zoom_at(
            Point {
                x: geometry.container.width / 2.0,
                y: geometry.container.height / 2.0,
            },
            factor,
            geometry.limits.min_zoom,
            geometry.limits.max_zoom,
        );
        drop(geometry);
        self.viewport.set(viewport);
    }
    pub fn fit_view(mut self) {
        let geometry = self.geometry.peek();
        let bounds = geometry.rects.values().copied().reduce(|a, b| a.union(b));
        if let Some(bounds) = bounds {
            let viewport = Viewport::fit(
                bounds,
                geometry.container,
                FIT_PADDING,
                geometry.limits.min_zoom,
                geometry.limits.max_zoom,
            );
            drop(geometry);
            self.viewport.set(viewport);
        }
    }
    pub fn center_on(mut self, id: NodeId) {
        let geometry = self.geometry.peek();
        if let Some(rect) = geometry.rects.get(&id) {
            let p = rect.center();
            let zoom = self.viewport.peek().zoom;
            let viewport = Viewport {
                x: geometry.container.width / 2.0 - p.x * zoom,
                y: geometry.container.height / 2.0 - p.y * zoom,
                zoom,
            };
            drop(geometry);
            self.viewport.set(viewport);
        }
    }
    pub fn relayout(mut self) {
        let next = *self.revision.peek() + 1;
        self.revision.set(next);
    }
}

#[component]
pub fn GraphControls(controller: GraphController) -> Element {
    let on_zoom_in = move |_| controller.zoom_by(1.2);
    let on_zoom_out = move |_| controller.zoom_by(1.0 / 1.2);
    let on_fit = move |_| controller.fit_view();
    let on_relayout = move |_| controller.relayout();
    rsx! {
        div {
            class: "dxgraph-controls",
            role: "group",
            aria_label: "Graph controls",
            dxcomp::button::Button { aria_label: "Zoom in", onclick: on_zoom_in, "+" }
            dxcomp::button::Button { aria_label: "Zoom out", onclick: on_zoom_out, "−" }
            dxcomp::button::Button { aria_label: "Fit graph", onclick: on_fit, "Fit" }
            dxcomp::button::Button { aria_label: "Re-layout graph", onclick: on_relayout, "Re-layout" }
        }
    }
}
