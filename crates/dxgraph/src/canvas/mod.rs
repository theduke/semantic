mod background;
mod controls;
mod edges;
mod keyboard;
mod node;
mod origin;
mod pipeline;
mod state;
use background::GraphBackground;
pub use controls::{GraphController, GraphControls, use_graph_controller};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeDetail {
    #[default]
    Full,
    Compact,
    Minimal,
}

use crate::{
    GraphModel, NodeId, Point, Rect, Size, ViewportLimits,
    interaction::{
        GestureContext, GestureEffect, GestureInput, GestureState, GestureTarget, WheelMode, input,
        platform,
    },
    layout::LayoutConfig,
};
use dioxus::prelude::*;
use indexmap::{IndexMap, IndexSet};
use keyboard::{KeyboardAction, keyboard_action};
use origin::{NativeInputs, OriginCache, OriginRefresh};
use pipeline::{CanvasEffect, CanvasInput, CanvasPipeline};
use state::SelectionEvent;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasConfig {
    pub limits: ViewportLimits,
    pub full_detail_zoom: f64,
    pub compact_detail_zoom: f64,
    pub cull_margin: f64,
    pub wheel_mode: WheelMode,
    pub aria_label: String,
}
impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            limits: ViewportLimits::default(),
            full_detail_zoom: 0.6,
            compact_detail_zoom: 0.3,
            cull_margin: 200.0,
            wheel_mode: WheelMode::Zoom,
            aria_label: "Graph canvas".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct NodeRenderContext<N> {
    pub id: NodeId,
    pub data: N,
    pub selected: bool,
    pub detail: NodeDetail,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NodeEvent {
    pub id: NodeId,
    pub shift: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct GraphCanvasProps<N: Clone + PartialEq + 'static, E: Clone + PartialEq + 'static> {
    pub model: ReadSignal<GraphModel<N, E>>,
    pub render_node: Callback<NodeRenderContext<N>, Element>,
    pub node_label: Callback<NodeRenderContext<N>, String>,
    #[props(default)]
    pub render_minimal: Option<Callback<NodeRenderContext<N>, Element>>,
    #[props(default)]
    pub layout: LayoutConfig,
    #[props(default)]
    pub config: CanvasConfig,
    #[props(default)]
    pub controller: Option<GraphController>,
    #[props(default)]
    pub on_node_click: Option<EventHandler<NodeEvent>>,
    #[props(default)]
    pub on_node_activate: Option<EventHandler<NodeEvent>>,
    #[props(default)]
    pub on_node_moved: Option<EventHandler<(NodeId, Point)>>,
    #[props(default)]
    pub on_selection_change: Option<EventHandler<Vec<NodeId>>>,
    #[props(default)]
    pub on_background_click: Option<EventHandler<Point>>,
    #[props(default)]
    pub class: Option<String>,
    #[props(default)]
    pub children: Element,
}

/// Apply reducer effects in one place, keeping controller geometry in sync.
fn apply_canvas_input<N: Clone + PartialEq + 'static, E: Clone + PartialEq + 'static>(
    mut pipeline: Signal<CanvasPipeline>,
    model: ReadSignal<GraphModel<N, E>>,
    mut controller: GraphController,
    config: CanvasConfig,
    input: CanvasInput,
) {
    if controller.geometry.peek().limits != config.limits {
        controller.geometry.write().limits = config.limits;
    }
    let effects = pipeline.write().reduce(&model.peek(), input);
    for effect in effects {
        match effect {
            CanvasEffect::RectsChanged => {
                let pipeline = pipeline.peek();
                let mut geometry = controller.geometry.write();
                geometry.rects = pipeline.rects.clone();
                geometry.revision = pipeline.revision;
            }
            CanvasEffect::ContainerChanged => {
                if let Some(container) = pipeline.peek().container {
                    controller.geometry.write().container = container;
                }
            }
            CanvasEffect::FitView => controller.fit_view(),
            CanvasEffect::ScheduleTimeout(batch) => {
                let config = config.clone();
                spawn(async move {
                    dioxus_sdk_time::sleep(std::time::Duration::from_millis(100)).await;
                    apply_canvas_input(
                        pipeline,
                        model,
                        controller,
                        config,
                        CanvasInput::MeasureTimeout(batch),
                    );
                });
            }
            CanvasEffect::ScheduleMeasurements(flush) => {
                let config = config.clone();
                spawn(async move {
                    // A single queued tick absorbs a burst of ResizeObserver notifications.
                    dioxus_sdk_time::sleep(std::time::Duration::from_millis(16)).await;
                    apply_canvas_input(
                        pipeline,
                        model,
                        controller,
                        config,
                        CanvasInput::FlushMeasurements(flush),
                    );
                });
            }
        }
    }
}

type EdgeGroups = IndexMap<(NodeId, NodeId), Vec<(crate::EdgeId, bool)>>;

/// Lane indices use half-spacing units so even-sized groups are centred too.
fn parallel_edge_offsets<N, E>(model: &GraphModel<N, E>) -> IndexMap<crate::EdgeId, i32> {
    let mut groups = EdgeGroups::new();
    for edge in model.edges() {
        let reversed = edge.source > edge.target;
        let pair = if reversed {
            (edge.target.clone(), edge.source.clone())
        } else {
            (edge.source.clone(), edge.target.clone())
        };
        groups
            .entry(pair)
            .or_default()
            .push((edge.id.clone(), reversed));
    }
    let mut offsets = IndexMap::new();
    for (pair, group) in &groups {
        let count = group.len() as i32;
        for (index, (edge, reversed)) in group.iter().enumerate() {
            let lane = if pair.0 == pair.1 {
                index as i32 * 2
            } else {
                index as i32 * 2 - (count - 1)
            };
            // Geometry computes its perpendicular from the directed edge.
            offsets.insert(edge.clone(), if *reversed { -lane } else { lane });
        }
    }
    offsets
}

#[derive(Clone, Default, PartialEq)]
struct CanvasVisibility {
    region: Option<(Rect, f64)>,
    data: Option<(u64, IndexSet<NodeId>, Option<NodeId>)>,
    visible: IndexSet<NodeId>,
    detail: NodeDetail,
}

#[allow(non_snake_case)]
pub fn GraphCanvas<N: Clone + PartialEq + 'static, E: Clone + PartialEq + 'static>(
    props: GraphCanvasProps<N, E>,
) -> Element {
    let fallback = use_graph_controller();
    let controller = props.controller.unwrap_or(fallback);
    let model = props.model;
    let pipeline = use_signal(|| {
        CanvasPipeline::new(
            &model.peek(),
            props.layout.clone(),
            *controller.revision.peek(),
            *controller.reset_revision.peek(),
        )
    });
    let mut gesture = use_signal(GestureState::default);
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let config = props.config.clone();
    let apply = use_callback(move |input| {
        apply_canvas_input(pipeline, model, controller, config.clone(), input);
    });
    use_effect(move || apply.call(CanvasInput::Start));
    let layout = props.layout.clone();
    let config = props.config.clone();
    use_effect(use_reactive!(|layout, config| {
        let _ = config; // Config changes must synchronize controller limits as well.
        let _current = model.read();
        let revision = (controller.revision)();
        let reset_revision = (controller.reset_revision)();
        apply.call(CanvasInput::ModelChanged);
        apply.call(CanvasInput::LayoutChanged(layout.clone(), revision));
        apply.call(CanvasInput::ResetPositions(reset_revision));
    }));
    let offsets = use_memo(move || parallel_edge_offsets(&model.read()));
    let mut visibility = use_signal(|| CanvasVisibility {
        visible: model.peek().nodes().map(|node| node.id.clone()).collect(),
        ..CanvasVisibility::default()
    });
    let cull_config = props.config.clone();
    use_effect(use_reactive!(|cull_config| {
        let viewport = *controller.viewport.read();
        let geometry = controller.geometry.read();
        let pipeline = pipeline.read();
        let mut next = visibility.peek().clone();
        let view = viewport.visible_world_rect(geometry.container);
        let recull = next
            .region
            .is_none_or(|(rect, zoom)| state::needs_recull(rect, view, zoom, viewport.zoom));
        if recull {
            next.region = Some((view.inflate(cull_config.cull_margin), viewport.zoom));
        }
        let data = (
            pipeline.revision,
            pipeline.state.selection.clone(),
            pipeline.state.dragging.clone(),
        );
        if recull || next.data.as_ref() != Some(&data) {
            let region = next.region.map(|(rect, _)| rect).unwrap_or(view);
            next.visible = state::visible_nodes(
                &pipeline.rects,
                region,
                &pipeline.state.selection,
                pipeline.state.dragging.as_ref(),
            );
            next.data = Some(data);
        }
        next.detail = state::detail_for_zoom(
            viewport.zoom,
            cull_config.full_detail_zoom,
            cull_config.compact_detail_zoom,
        );
        if *visibility.peek() != next {
            visibility.set(next);
        }
    }));
    let callbacks = props.clone();
    let dispatch = use_callback(move |(event, origin): (GestureInput, Point)| {
        let positions = |id: &NodeId| pipeline.peek().state.positions.get(id).copied();
        let context = GestureContext {
            viewport: controller.viewport(),
            container_origin: origin,
            limits: callbacks.config.limits,
            wheel_mode: callbacks.config.wheel_mode,
            node_position: &positions,
        };
        let effects = gesture.write().handle(event, &context);
        for effect in effects {
            if matches!(
                &effect,
                GestureEffect::NodeDragStart(_)
                    | GestureEffect::NodeDragMove { .. }
                    | GestureEffect::NodeDragEnd { .. }
                    | GestureEffect::NodeClick { .. }
                    | GestureEffect::BackgroundClick { .. }
            ) {
                apply.call(CanvasInput::Gesture(effect.clone()));
            }
            match effect {
                GestureEffect::SetViewport(viewport) => controller.set_viewport(viewport),
                GestureEffect::NodeDragStart(_) | GestureEffect::NodeDragMove { .. } => {}
                GestureEffect::NodeDragEnd { node, position } => {
                    if let Some(callback) = callbacks.on_node_moved {
                        callback.call((node, position));
                    }
                }
                GestureEffect::NodeClick { node, shift } => {
                    if let Some(callback) = callbacks.on_selection_change {
                        callback.call(pipeline.peek().state.selection.iter().cloned().collect());
                    }
                    if let Some(callback) = callbacks.on_node_click {
                        callback.call(NodeEvent { id: node, shift });
                    }
                }
                GestureEffect::NodeDoubleClick(node) => {
                    if let Some(callback) = callbacks.on_node_activate {
                        callback.call(NodeEvent {
                            id: node,
                            shift: false,
                        });
                    }
                }
                GestureEffect::BackgroundClick { world } => {
                    if let Some(callback) = callbacks.on_selection_change {
                        callback.call(Vec::new());
                    }
                    if let Some(callback) = callbacks.on_background_click {
                        callback.call(world);
                    }
                }
                GestureEffect::CapturePointer(pointer) => {
                    platform::capture(mounted.peek().as_deref(), pointer)
                }
                GestureEffect::ReleasePointer(pointer) => {
                    platform::release(mounted.peek().as_deref(), pointer)
                }
            }
        }
    });
    let origin = use_hook(|| Rc::new(RefCell::new(OriginCache::default())));
    let native = use_hook(|| Rc::new(RefCell::new(NativeInputs::default())));
    let resized = use_callback(move |size| {
        if pipeline.peek().container != Some(size) {
            apply.call(CanvasInput::ContainerResized(size));
        }
    });
    let refresh_origin = use_callback({
        let refresh = OriginRefresh {
            cache: origin.clone(),
            inputs: native.clone(),
            mounted,
            gesture,
            resized,
            dispatch,
        };
        move |()| refresh.request()
    });
    use_hook(move || Rc::new(platform::listen_for_scroll(refresh_origin)));
    let send_input = use_callback({
        let origin = origin.clone();
        let native = native.clone();
        move |event: GestureInput| {
            #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
            {
                let needs_origin = gesture
                    .peek()
                    .requires_fresh_origin(&event, props.config.wheel_mode);
                let gated = native.borrow().gated();
                if needs_origin || gated {
                    native.borrow_mut().push(event);
                    if needs_origin {
                        refresh_origin.call(());
                    }
                    return;
                }
            }
            #[cfg(all(feature = "web", target_arch = "wasm32"))]
            let _ = &native;
            if OriginCache::refresh_for_input(&event, props.config.wheel_mode) {
                refresh_origin.call(());
            }
            // Ordinary pan/drag input uses the cache unless an earlier input is awaiting bounds.
            let cached = origin.borrow().point();
            dispatch.call((event, cached));
        }
    });
    let activate = use_callback(move |event: NodeEvent| {
        if let Some(callback) = props.on_node_activate {
            callback.call(event);
        }
    });
    let select = use_callback(move |event: NodeEvent| {
        apply.call(CanvasInput::Selection(SelectionEvent::Click {
            node: event.id,
            shift: event.shift,
        }));
        if let Some(callback) = props.on_selection_change {
            callback.call(pipeline.peek().state.selection.iter().cloned().collect());
        }
    });
    let node_pointer = use_callback(move |(id, event): (NodeId, PointerEvent)| {
        send_input.call(input::pointer_down(
            &event,
            GestureTarget::Node(id),
            platform::timestamp_ms(),
        ));
    });
    let measure = use_callback(move |(id, size, detail): (NodeId, Size, NodeDetail)| {
        if detail == NodeDetail::Full {
            apply.call(CanvasInput::Measured { id, size, detail });
        }
    });
    let on_mounted = move |event: MountedEvent| {
        let element = event.data();
        mounted.set(Some(element));
        refresh_origin.call(());
    };
    let on_resize = move |event: ResizeEvent| {
        if let Ok(size) = event.get_border_box_size() {
            apply.call(CanvasInput::ContainerResized(Size::new(
                size.width,
                size.height,
            )));
        }
        refresh_origin.call(());
    };
    let on_pointer_down = move |event: PointerEvent| {
        send_input.call(input::pointer_down(
            &event,
            GestureTarget::Background,
            platform::timestamp_ms(),
        ));
    };
    let on_pointer_move = move |event: PointerEvent| send_input.call(input::pointer_move(&event));
    let on_pointer_up = move |event: PointerEvent| {
        send_input.call(input::pointer_up(&event, platform::timestamp_ms()))
    };
    let on_pointer_cancel = move |event: PointerEvent| {
        send_input.call(GestureInput::PointerCancel {
            pointer: event.pointer_id(),
        })
    };
    let on_pointer_leave = move |event: PointerEvent| {
        #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
        send_input.call(GestureInput::PointerCancel {
            pointer: event.pointer_id(),
        });
        #[cfg(all(feature = "web", target_arch = "wasm32"))]
        let _ = event;
    };
    let on_wheel = move |event: WheelEvent| {
        event.prevent_default();
        send_input.call(input::wheel(&event, controller.geometry.peek().container));
    };
    let on_key_down = move |event: KeyboardEvent| {
        let handled = match keyboard_action(&event.key().to_string()) {
            Some(KeyboardAction::ClearSelection) => {
                apply.call(CanvasInput::Selection(SelectionEvent::Clear));
                if let Some(callback) = props.on_selection_change {
                    callback.call(Vec::new());
                }
                true
            }
            Some(KeyboardAction::ZoomBy(factor)) => {
                controller.zoom_by(factor);
                true
            }
            Some(KeyboardAction::Fit) => {
                controller.fit_view();
                true
            }
            Some(KeyboardAction::Pan(delta)) => {
                let mut viewport = controller.viewport();
                viewport.x += delta.x;
                viewport.y += delta.y;
                controller.set_viewport(viewport);
                true
            }
            _ => false,
        };
        if handled {
            event.prevent_default();
        }
    };
    let current = model.read();
    let pipeline = pipeline.read();
    let visibility = visibility.read();
    let offsets = offsets.read();
    let marker = use_hook(|| format!("dxgraph-arrow-{}", dioxus::core::current_scope_id().0));
    let class = format!("dxgraph {}", props.class.as_deref().unwrap_or(""));
    let world = rsx! {
        svg { class: "dxgraph-edges", "aria-hidden": "true",
            defs {
                marker {
                    id: marker.clone(),
                    view_box: "0 0 10 10",
                    ref_x: "9",
                    ref_y: "5",
                    marker_width: "6",
                    marker_height: "6",
                    orient: "auto-start-reverse",
                    path { d: "M 0 0 L 10 5 L 0 10 z", fill: "currentColor" }
                }
            }
            for edge in current
                .edges()
                .filter(|edge| state::edge_visible(
                    &edge.source,
                    &edge.target,
                    &visibility.visible,
                ))
            {
                if let (Some(source), Some(target)) = (
                    pipeline.rects.get(&edge.source),
                    pipeline.rects.get(&edge.target),
                )
                {
                    edges::EdgeView {
                        key: "{edge.id}",
                        id: edge.id.to_string(),
                        source: *source,
                        target: *target,
                        self_loop: edge.source == edge.target,
                        style: edge.style.clone(),
                        label: edge.label.clone(),
                        marker: marker.clone(),
                        offset: offsets.get(&edge.id).copied().unwrap_or_default(),
                    }
                }
            }
        }
        for graph_node in current
            .nodes()
            .filter(|node| {
                visibility.visible.contains(&node.id)
                    || pipeline.state.pending.contains(&node.id)
            })
        {
            node::NodeView {
                key: "{graph_node.id}",
                id: graph_node.id.clone(),
                data: graph_node.data.clone(),
                position: pipeline.rects.get(&graph_node.id).map(|rect| rect.origin).unwrap_or_default(),
                size: pipeline.state.measured.get(&graph_node.id).copied().unwrap_or(graph_node.size_hint),
                selected: pipeline.state.selection.contains(&graph_node.id),
                hidden: pipeline.state.pending.contains(&graph_node.id),
                detail: if pipeline.state.pending.contains(&graph_node.id) { NodeDetail::Full } else { visibility.detail },
                label: props
                    .node_label
                    .call(NodeRenderContext {
                        id: graph_node.id.clone(),
                        data: graph_node.data.clone(),
                        selected: pipeline.state.selection.contains(&graph_node.id),
                        detail: visibility.detail,
                    }),
                render_node: props.render_node,
                render_minimal: props.render_minimal,
                on_pointer: node_pointer,
                on_measure: measure,
                on_activate: activate,
                on_select: select,
            }
        }
    };
    rsx! {
        div {
            class,
            role: "application",
            aria_label: props.config.aria_label.clone(),
            tabindex: 0,
            onmounted: on_mounted,
            onresize: on_resize,
            onpointerdown: on_pointer_down,
            onpointermove: on_pointer_move,
            onpointerup: on_pointer_up,
            onpointercancel: on_pointer_cancel,
            onpointerleave: on_pointer_leave,
            onwheel: on_wheel,
            onkeydown: on_key_down,
            GraphBackground { viewport: controller.viewport }
            background::GraphWorld { viewport: controller.viewport, children: world }
            div { class: "dxgraph-overlay", {props.children} }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::pipeline::{calculate_layout, rects};
    use super::state::CanvasState;
    use super::*;
    use crate::GraphNode;

    #[test]
    fn measured_expansion_keeps_old_nodes_and_places_new_nodes_without_overlap() {
        let mut model = GraphModel::<(), ()>::default();
        model.insert_node(GraphNode::new("root", ())).unwrap();
        let mut first = GraphNode::new("first", ());
        first.layout_parent = Some("root".into());
        model.insert_node(first).unwrap();
        let mut state = CanvasState::default();
        state.measured.insert(
            "root".into(),
            Size {
                width: 240.0,
                height: 70.0,
            },
        );
        state.measured.insert(
            "first".into(),
            Size {
                width: 240.0,
                height: 90.0,
            },
        );
        calculate_layout(&model, &mut state, &LayoutConfig::default(), false);
        let previous = state.positions.clone();
        state.stable = previous.keys().cloned().collect();
        let mut second = GraphNode::new("second", ());
        second.layout_parent = Some("root".into());
        model.insert_node(second).unwrap();
        calculate_layout(&model, &mut state, &LayoutConfig::default(), true);
        state.measured.insert(
            "second".into(),
            Size {
                width: 400.0,
                height: 130.0,
            },
        );
        calculate_layout(&model, &mut state, &LayoutConfig::default(), true);
        for (id, point) in previous {
            assert_eq!(state.positions[&id], point);
        }
        let rects = rects(&model, &state);
        let new = rects[&NodeId::from("second")];
        for id in ["root", "first"] {
            assert!(!new.intersects(rects[&NodeId::from(id)]));
        }
    }

    #[test]
    fn partial_measurements_use_received_sizes_and_hints_together() {
        let mut model = GraphModel::<(), ()>::default();
        model.insert_node(GraphNode::new("a", ())).unwrap();
        model.insert_node(GraphNode::new("b", ())).unwrap();
        let mut state = CanvasState::default();
        state.measured.insert(
            "a".into(),
            Size {
                width: 400.0,
                height: 200.0,
            },
        );
        calculate_layout(&model, &mut state, &LayoutConfig::default(), false);
        let rects = rects(&model, &state);
        assert_eq!(rects[&NodeId::from("a")].size.width, 400.0);
        assert_eq!(
            rects[&NodeId::from("b")].size,
            model.node(&"b".into()).unwrap().size_hint
        );
        assert!(!rects[&NodeId::from("a")].intersects(rects[&NodeId::from("b")]));
    }
    #[test]
    fn parallel_and_opposite_edges_have_centred_distinct_lanes() {
        let mut model = GraphModel::<(), ()>::default();
        model.insert_node(GraphNode::new("a", ())).unwrap();
        model.insert_node(GraphNode::new("b", ())).unwrap();
        model
            .insert_edge(crate::GraphEdge::new("ab", "a", "b", ()))
            .unwrap();
        model
            .insert_edge(crate::GraphEdge::new("ba", "b", "a", ()))
            .unwrap();
        let offsets = parallel_edge_offsets(&model);
        assert_eq!(offsets[&crate::EdgeId::from("ab")], -1);
        assert_eq!(offsets[&crate::EdgeId::from("ba")], -1);
        let a = Rect::new(Point::default(), Size::new(100.0, 60.0));
        let b = Rect::new(Point::new(300.0, 0.0), a.size);
        let forward = crate::edge_geometry_with_offset(
            a,
            b,
            &crate::EdgeStyle::default(),
            offsets[&crate::EdgeId::from("ab")],
            false,
        );
        let reverse = crate::edge_geometry_with_offset(
            b,
            a,
            &crate::EdgeStyle::default(),
            offsets[&crate::EdgeId::from("ba")],
            false,
        );
        assert_ne!(
            forward.label_pos, reverse.label_pos,
            "opposite edges must occupy different physical lanes"
        );
        model
            .insert_edge(crate::GraphEdge::new("ab2", "a", "b", ()))
            .unwrap();
        let offsets = parallel_edge_offsets(&model);
        assert_eq!(offsets[&crate::EdgeId::from("ab")], -2);
        assert_eq!(offsets[&crate::EdgeId::from("ba")], 0);
        assert_eq!(offsets[&crate::EdgeId::from("ab2")], 2);
    }

    #[test]
    fn self_loop_offsets_have_distinct_radii() {
        let mut model = GraphModel::<(), ()>::default();
        model.insert_node(GraphNode::new("a", ())).unwrap();
        for id in ["one", "two", "three"] {
            model
                .insert_edge(crate::GraphEdge::new(id, "a", "a", ()))
                .unwrap();
        }
        let offsets = parallel_edge_offsets(&model);
        assert_eq!(offsets.values().copied().collect::<Vec<_>>(), [0, 2, 4]);
    }
}
