use super::{
    NodeDetail, NodeEvent, NodeRenderContext,
    keyboard::{KeyboardAction, keyboard_action},
};
use crate::{NodeId, Point, Size};
use dioxus::prelude::*;

fn node_border_box(event: &ResizeEvent) -> Option<Size> {
    // Native resize events can bubble. A node's size must never replace the
    // container dimensions used by fit, center, and culling.
    event.stop_propagation();
    event.get_border_box_size().ok().map(|size| Size {
        width: size.width,
        height: size.height,
    })
}

#[derive(Props, Clone, PartialEq)]
pub(crate) struct NodeViewProps<N: Clone + PartialEq + 'static> {
    pub id: NodeId,
    pub data: N,
    pub position: Point,
    pub size: Size,
    pub selected: bool,
    pub hidden: bool,
    pub detail: NodeDetail,
    pub label: String,
    pub render_node: Callback<NodeRenderContext<N>, Element>,
    pub render_minimal: Option<Callback<NodeRenderContext<N>, Element>>,
    pub on_pointer: EventHandler<(NodeId, PointerEvent)>,
    pub on_measure: EventHandler<(NodeId, Size, NodeDetail)>,
    pub on_activate: EventHandler<NodeEvent>,
    pub on_select: EventHandler<NodeEvent>,
}

#[allow(non_snake_case)]
pub(crate) fn NodeView<N: Clone + PartialEq + 'static>(props: NodeViewProps<N>) -> Element {
    let context = NodeRenderContext {
        id: props.id.clone(),
        data: props.data,
        selected: props.selected,
        detail: props.detail,
    };
    let content = if props.detail == NodeDetail::Minimal {
        if let Some(renderer) = props.render_minimal {
            renderer.call(context)
        } else {
            rsx! {
                div {
                    class: "dxgraph-placeholder",
                    style: "width:{props.size.width}px;height:{props.size.height}px",
                    aria_label: props.label.clone(),
                }
            }
        }
    } else {
        props.render_node.call(context)
    };
    let pointer_id = props.id.clone();
    let measure_id = props.id.clone();
    let key_id = props.id.clone();
    let on_pointer_down = move |event: PointerEvent| {
        event.stop_propagation();
        props.on_pointer.call((pointer_id.clone(), event));
    };
    let on_resize = move |event: ResizeEvent| {
        if let Some(size) = node_border_box(&event) {
            props
                .on_measure
                .call((measure_id.clone(), size, props.detail));
        }
    };
    let on_key_down = move |event: KeyboardEvent| {
        let callback = match keyboard_action(&event.key().to_string()) {
            Some(KeyboardAction::Activate) => props.on_activate,
            Some(KeyboardAction::Select) => props.on_select,
            _ => return,
        };
        event.stop_propagation();
        event.prevent_default();
        callback.call(NodeEvent {
            id: key_id.clone(),
            shift: event.modifiers().shift(),
        });
    };
    let style = format!(
        "transform:translate({}px,{}px);visibility:{}",
        props.position.x,
        props.position.y,
        if props.hidden { "hidden" } else { "visible" }
    );
    rsx! {
        div {
            class: "dxgraph-node",
            "data-dxgraph-node": props.id.to_string(),
            "data-selected": props.selected.to_string(),
            tabindex: 0,
            role: "button",
            aria_label: props.label,
            style,
            onpointerdown: on_pointer_down,
            onresize: on_resize,
            onkeydown: on_key_down,
            {content}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::html::{HasResizeData, ResizeError, geometry::PixelsSize};
    struct NodeResize;
    impl HasResizeData for NodeResize {
        fn get_border_box_size(&self) -> Result<PixelsSize, ResizeError> {
            Ok(PixelsSize::new(160.0, 64.0))
        }
        fn get_content_box_size(&self) -> Result<PixelsSize, ResizeError> {
            Ok(PixelsSize::new(140.0, 44.0))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    #[test]
    fn node_resize_measures_border_box_without_reaching_container() {
        let event = Event::new(std::rc::Rc::new(ResizeData::new(NodeResize)), true);
        assert!(event.propagates());
        assert_eq!(
            node_border_box(&event),
            Some(Size {
                width: 160.0,
                height: 64.0
            })
        );
        assert!(
            !event.propagates(),
            "A node resize must not reach the canvas resize handler"
        );
    }
}
