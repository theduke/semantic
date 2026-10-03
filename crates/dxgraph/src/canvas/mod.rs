mod background;
mod controls;
mod edges;
pub mod keyboard;
mod node;
pub mod state;
pub use background::GraphBackground;
pub use controls::{GraphController, GraphControls, use_graph_controller};
pub use state::NodeDetail;

use crate::{
    GraphModel, NodeId, Point, Rect, Size, ViewportLimits,
    interaction::{
        GestureContext, GestureEffect, GestureInput, GestureState, GestureTarget, WheelMode, input,
        platform,
    },
    layout::{LayoutConfig, LayoutEdge, LayoutInput, LayoutNode, run_layout},
};
use dioxus::prelude::*;
use indexmap::{IndexMap, IndexSet};
use keyboard::{KeyboardAction, keyboard_action};
use state::{CanvasState, SelectionEvent, reduce_selection};
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
    pub node_label: Callback<NodeId, String>,
    #[props(default)]
    pub render_minimal: Option<Callback<NodeRenderContext<N>, Element>>,
    #[props(default)]
    pub layout: LayoutConfig,
    #[props(default)]
    pub layout_revision: u64,
    #[props(default)]
    pub config: CanvasConfig,
    #[props(default)]
    pub controller: Option<GraphController>,
    #[props(default)]
    pub on_node_click: Option<EventHandler<NodeEvent>>,
    #[props(default)]
    pub on_node_double_click: Option<EventHandler<NodeEvent>>,
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

fn rects<N, E>(model: &GraphModel<N, E>, state: &CanvasState) -> IndexMap<NodeId, Rect> {
    model
        .nodes()
        .filter_map(|node| {
            state
                .positions
                .get(&node.id)
                .or(node.position.as_ref())
                .map(|p| {
                    (
                        node.id.clone(),
                        Rect {
                            origin: *p,
                            size: state
                                .measured
                                .get(&node.id)
                                .copied()
                                .unwrap_or(node.size_hint),
                        },
                    )
                })
        })
        .collect()
}

fn calculate_layout<N, E>(
    model: &GraphModel<N, E>,
    state: &mut CanvasState,
    layout: &LayoutConfig,
    preserve: bool,
) {
    let input = LayoutInput {
        nodes: model
            .nodes()
            .map(|node| LayoutNode {
                id: node.id.clone(),
                size: state
                    .measured
                    .get(&node.id)
                    .copied()
                    .unwrap_or(node.size_hint),
                fixed: if node.pinned {
                    node.position
                        .or_else(|| state.positions.get(&node.id).copied())
                } else if state.moved.contains(&node.id)
                    || (preserve && state.stable.contains(&node.id))
                {
                    state.positions.get(&node.id).copied()
                } else {
                    None
                },
                previous: if state.stable.contains(&node.id) || state.moved.contains(&node.id) {
                    state.positions.get(&node.id).copied().or(node.position)
                } else {
                    node.position
                },
                layout_parent: node.layout_parent.clone(),
                order_key: node.order_key.clone(),
            })
            .collect(),
        edges: model
            .edges()
            .map(|edge| LayoutEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                weight: 1.0,
            })
            .collect(),
        roots: model
            .nodes()
            .filter(|node| node.layout_parent.is_none())
            .map(|node| node.id.clone())
            .collect(),
    };
    state.merge_positions(model, run_layout(layout, &input).positions);
}

#[allow(non_snake_case)]
pub fn GraphCanvas<N: Clone + PartialEq + 'static, E: Clone + PartialEq + 'static>(
    props: GraphCanvasProps<N, E>,
) -> Element {
    let fallback = use_graph_controller();
    let mut controller = props.controller.unwrap_or(fallback);
    let model = props.model;
    let mut state = use_signal(|| {
        let mut s = CanvasState::default();
        calculate_layout(&model.peek(), &mut s, &props.layout, false);
        s.pending = model.peek().nodes().map(|node| node.id.clone()).collect();
        s
    });
    let mut gesture = use_signal(GestureState::default);
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let mut origin = use_signal(Point::default);
    let mut fitted = use_signal(|| false);
    let mut measured_once = use_signal(|| false);
    let mut measurement_batch = use_signal(|| 1u64);
    let mut signature = use_signal(|| state::layout_signature(&model.peek()));
    let mut layout_key = use_signal(|| (props.layout.clone(), props.layout_revision, 0u64));
    let layout = props.layout.clone();
    let layout_revision = props.layout_revision;
    let config = props.config.clone();
    // Reactive inputs drive layout; node positions and payload changes are excluded.
    use_effect(use_reactive!(|layout, layout_revision, config| {
        let current = model.read();
        let next = state::layout_signature(&current);
        let revision = (controller.revision)();
        let key = (layout.clone(), layout_revision, revision);
        let changed = *signature.peek() != next;
        let forced = *layout_key.peek() != key;
        if changed || forced {
            let mut s = state.write();
            s.stable = if forced {
                IndexSet::new()
            } else {
                s.positions.keys().cloned().collect()
            };
            s.pending = current
                .nodes()
                .filter(|node| !s.measured.contains_key(&node.id))
                .map(|node| node.id.clone())
                .collect();
            calculate_layout(&current, &mut s, &layout, changed && !forced);
            let pending = !s.pending.is_empty();
            drop(s);
            if pending {
                let next = *measurement_batch.peek() + 1;
                measurement_batch.set(next);
            }
            signature.set(next);
            layout_key.set(key);
        }
        let s = state.peek();
        let next_rects = rects(&current, &s);
        drop(s);
        let mut geometry = controller.geometry;
        let mut value = geometry.peek().clone();
        value.rects = next_rects;
        value.revision += 1;
        value.limits = config.limits;
        geometry.set(value);
    }));
    // A hint is enough for initial rendering; reveal stragglers after a bounded wait.
    let timeout_layout = props.layout.clone();
    use_effect(move || {
        let batch = measurement_batch();
        if state.peek().pending.is_empty() {
            return;
        }
        let layout = timeout_layout.clone();
        spawn(async move {
            dioxus_sdk_time::sleep(std::time::Duration::from_millis(100)).await;
            if *measurement_batch.peek() == batch {
                let mut s = state.write();
                let current = model.peek();
                calculate_layout(&current, &mut s, &layout, *measured_once.peek());
                s.pending.clear();
                s.stable = s.positions.keys().cloned().collect();
                let rects = rects(&current, &s);
                drop(current);
                drop(s);
                measured_once.set(true);
                {
                    let mut geometry = controller.geometry.write();
                    geometry.rects = rects;
                    geometry.revision += 1;
                }
                if !*fitted.peek() && mounted.peek().is_some() {
                    controller.fit_view();
                    fitted.set(true);
                }
            }
        });
    });
    let mut cull = use_signal(|| None::<(Rect, f64)>);
    let mut cull_data = use_signal(|| None::<(u64, IndexSet<NodeId>, Option<NodeId>)>);
    let mut visible = use_signal(|| {
        model
            .peek()
            .nodes()
            .map(|node| node.id.clone())
            .collect::<IndexSet<_>>()
    });
    let mut detail = use_signal(|| NodeDetail::Full);
    let cull_config = props.config.clone();
    use_effect(use_reactive!(|cull_config| {
        let viewport = *controller.viewport.read();
        let geometry = controller.geometry.read();
        let s = state.read();
        let view = viewport.visible_world_rect(geometry.container);
        let should = (*cull.peek())
            .is_none_or(|(rect, zoom)| state::needs_recull(rect, view, zoom, viewport.zoom));
        if should {
            let inflated = view.inflate(cull_config.cull_margin);
            cull.set(Some((inflated, viewport.zoom)));
        }
        let region = (*cull.peek()).map(|(rect, _)| rect).unwrap_or(view);
        let data = (geometry.revision, s.selection.clone(), s.dragging.clone());
        if should || cull_data.peek().as_ref() != Some(&data) {
            let next =
                state::visible_nodes(&geometry.rects, region, &s.selection, s.dragging.as_ref());
            if *visible.peek() != next {
                visible.set(next);
            }
            cull_data.set(Some(data));
        }
        let next = state::detail_for_zoom(
            viewport.zoom,
            cull_config.full_detail_zoom,
            cull_config.compact_detail_zoom,
        );
        if *detail.peek() != next {
            detail.set(next);
        }
    }));
    let callbacks = props.clone();
    let dispatch = use_callback(move |event: GestureInput| {
        let positions = |id: &NodeId| state.peek().positions.get(id).copied();
        let context = GestureContext {
            viewport: controller.viewport(),
            container_origin: *origin.peek(),
            limits: callbacks.config.limits,
            wheel_mode: callbacks.config.wheel_mode,
            node_position: &positions,
        };
        let effects = gesture.write().handle(event, &context);
        let mut positions_changed = false;
        for effect in effects {
            if matches!(
                &effect,
                GestureEffect::NodeDragStart(_)
                    | GestureEffect::NodeDragMove { .. }
                    | GestureEffect::NodeDragEnd { .. }
                    | GestureEffect::NodeClick { .. }
                    | GestureEffect::BackgroundClick { .. }
            ) {
                positions_changed |= matches!(
                    &effect,
                    GestureEffect::NodeDragMove { .. } | GestureEffect::NodeDragEnd { .. }
                );
                state.write().apply_effect(&effect);
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
                    let s = state.peek();
                    let selection = s.selection.iter().cloned().collect();
                    drop(s);
                    if let Some(callback) = callbacks.on_selection_change {
                        callback.call(selection);
                    }
                    if let Some(callback) = callbacks.on_node_click {
                        callback.call(NodeEvent { id: node, shift });
                    }
                }
                GestureEffect::NodeDoubleClick(node) => {
                    if let Some(callback) = callbacks
                        .on_node_double_click
                        .or(callbacks.on_node_activate)
                    {
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
        if positions_changed {
            let s = state.peek();
            let current = model.peek();
            let r = rects(&current, &s);
            drop(s);
            drop(current);
            let mut geometry = controller.geometry.write();
            geometry.rects = r;
            geometry.revision += 1;
        }
    });
    let refresh_origin = use_callback(move |_: ()| {
        if let Some(element) = mounted.peek().clone() {
            spawn(async move {
                if let Ok(rect) = element.get_client_rect().await {
                    origin.set(Point {
                        x: rect.origin.x,
                        y: rect.origin.y,
                    });
                }
            });
        }
    });
    let activate = use_callback(move |event: NodeEvent| {
        if let Some(callback) = props.on_node_activate.or(props.on_node_double_click) {
            callback.call(event);
        }
    });
    let node_pointer = use_callback(move |(id, event): (NodeId, PointerEvent)| {
        refresh_origin.call(());
        dispatch.call(input::pointer_down(
            &event,
            GestureTarget::Node(id),
            platform::timestamp_ms(),
        ));
    });
    let select = use_callback(move |event: NodeEvent| {
        let mut s = state.write();
        s.selection = reduce_selection(
            &s.selection,
            SelectionEvent::Click {
                node: event.id,
                shift: event.shift,
            },
        );
        let selection = s.selection.iter().cloned().collect();
        drop(s);
        if let Some(callback) = props.on_selection_change {
            callback.call(selection);
        }
    });
    let layout_measure = props.layout.clone();
    let measure = use_callback(move |(id, size): (NodeId, Size)| {
        let mut s = state.write();
        let was_pending = !s.pending.is_empty();
        let changed = s.measure(id, size);
        let complete = s.pending.is_empty();
        if changed && complete {
            let current = model.peek();
            calculate_layout(
                &current,
                &mut s,
                &layout_measure,
                was_pending && *measured_once.peek(),
            );
            measured_once.set(true);
            s.stable = s.positions.keys().cloned().collect();
        }
        let current = model.peek();
        let r = rects(&current, &s);
        drop(s);
        drop(current);
        {
            let mut geometry = controller.geometry.write();
            geometry.rects = r;
            geometry.revision += 1;
        }
        if complete && !*fitted.peek() && mounted.peek().is_some() {
            controller.fit_view();
            fitted.set(true);
        }
    });
    let current = model.read();
    let s = state.read();
    let shown = visible.read();
    let detail = detail();
    let all_rects = rects(&current, &s);
    let marker = use_hook(|| format!("dxgraph-arrow-{}", dioxus::core::current_scope_id().0));
    let class = format!("dxgraph {}", props.class.as_deref().unwrap_or(""));
    let world = rsx! {
        svg {class:"dxgraph-edges","aria-hidden":"true",defs {marker {id:marker.clone(),view_box:"0 0 10 10",ref_x:"9",ref_y:"5",marker_width:"6",marker_height:"6",orient:"auto-start-reverse",path{d:"M 0 0 L 10 5 L 0 10 z",fill:"currentColor"}}},
            for edge in current.edges().filter(|edge|state::edge_visible(&edge.source,&edge.target,&shown)) {
                if let (Some(source),Some(target))=(all_rects.get(&edge.source),all_rects.get(&edge.target)) {edges::EdgeView{key:"{edge.id}",id:edge.id.to_string(),source:*source,target:*target,style:edge.style.clone(),marker:marker.clone(),offset:current.edges().take_while(|other|other.id!=edge.id).filter(|other|other.source==edge.source&&other.target==edge.target).count() as i32}}
            }
        }
        for graph_node in current.nodes().filter(|node|shown.contains(&node.id)||s.pending.contains(&node.id)) {
            node::NodeView {key:"{graph_node.id}",id:graph_node.id.clone(),data:graph_node.data.clone(),position:all_rects.get(&graph_node.id).map(|r|r.origin).unwrap_or_default(),size:s.measured.get(&graph_node.id).copied().unwrap_or(graph_node.size_hint),selected:s.selection.contains(&graph_node.id),hidden:s.pending.contains(&graph_node.id),detail,label:props.node_label.call(graph_node.id.clone()),render_node:props.render_node,render_minimal:props.render_minimal,on_pointer:node_pointer,on_measure:measure,on_activate:activate,on_select:select}
        }
    };
    rsx! {div {
        class,role:"application",aria_label:props.config.aria_label.clone(),tabindex:0,
        onmounted:move |event|{let element=event.data();mounted.set(Some(element.clone()));spawn(async move{if let Ok(rect)=element.get_client_rect().await {origin.set(Point{x:rect.origin.x,y:rect.origin.y});{let mut geometry=controller.geometry.write();geometry.container=Size{width:rect.size.width,height:rect.size.height};geometry.revision+=1;}if state.peek().pending.is_empty(){controller.fit_view();fitted.set(true);}}});},
        onresize:move |event|{if let Ok(size)=event.get_border_box_size(){{let mut geometry=controller.geometry.write();geometry.container=Size{width:size.width,height:size.height};geometry.revision+=1;}if !*fitted.peek()&&state.peek().pending.is_empty(){controller.fit_view();fitted.set(true);}}refresh_origin.call(());},
        onpointerdown:move |event|{refresh_origin.call(());dispatch.call(input::pointer_down(&event,GestureTarget::Background,platform::timestamp_ms()));},
        onpointermove:move |event|dispatch.call(input::pointer_move(&event)),onpointerup:move |event|dispatch.call(input::pointer_up(&event,platform::timestamp_ms())),
        onpointercancel:move |event|dispatch.call(GestureInput::PointerCancel{pointer:event.pointer_id()}),
        onpointerleave:move |event|{#[cfg(not(all(feature="web",target_arch="wasm32")))] dispatch.call(GestureInput::PointerCancel{pointer:event.pointer_id()});#[cfg(all(feature="web",target_arch="wasm32"))] let _=event;},
        onwheel:move |event|{event.prevent_default();dispatch.call(input::wheel(&event,controller.geometry.peek().container));},
        onkeydown:move |event|{let handled=match keyboard_action(&event.key().to_string()){Some(KeyboardAction::ClearSelection)=>{state.write().selection.clear();if let Some(callback)=props.on_selection_change{callback.call(Vec::new());}true},Some(KeyboardAction::ZoomBy(factor))=>{controller.zoom_by(factor);true},Some(KeyboardAction::Fit)=>{controller.fit_view();true},Some(KeyboardAction::Pan(delta))=>{let mut viewport=controller.viewport();viewport.x+=delta.x;viewport.y+=delta.y;controller.set_viewport(viewport);true},_=>false};if handled{event.prevent_default();}},
        GraphBackground{viewport:controller.viewport}
        background::GraphWorld{viewport:controller.viewport,children:world}
        div{class:"dxgraph-overlay",{props.children}}
    }}
}

#[cfg(test)]
mod tests {
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
}
