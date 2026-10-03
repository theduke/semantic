use indexmap::{IndexMap, IndexSet};

pub use crate::geometry::NodeDetail;
use crate::{GraphModel, NodeId, Point, Rect, Size};

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasState {
    pub measured: IndexMap<NodeId, Size>,
    pub positions: IndexMap<NodeId, Point>,
    pub selection: IndexSet<NodeId>,
    pub moved: IndexSet<NodeId>,
    /// Nodes placed before the current measurement batch began.
    pub stable: IndexSet<NodeId>,
    pub dragging: Option<NodeId>,
    pub pending: IndexSet<NodeId>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            measured: IndexMap::new(),
            positions: IndexMap::new(),
            selection: IndexSet::new(),
            moved: IndexSet::new(),
            stable: IndexSet::new(),
            dragging: None,
            pending: IndexSet::new(),
        }
    }
}

impl CanvasState {
    /// Apply graph-local effects independently of the renderer and viewport.
    pub fn apply_effect(&mut self, effect: &crate::interaction::GestureEffect) {
        use crate::interaction::GestureEffect;
        match effect {
            GestureEffect::NodeDragStart(id) => self.dragging = Some(id.clone()),
            GestureEffect::NodeDragMove { node, position } => {
                self.positions.insert(node.clone(), *position);
            }
            GestureEffect::NodeDragEnd { node, position } => {
                self.dragging = None;
                self.moved.insert(node.clone());
                self.positions.insert(node.clone(), *position);
            }
            GestureEffect::NodeClick { node, shift } => {
                self.selection = reduce_selection(
                    &self.selection,
                    SelectionEvent::Click {
                        node: node.clone(),
                        shift: *shift,
                    },
                )
            }
            GestureEffect::BackgroundClick { .. } => self.selection.clear(),
            _ => {}
        }
    }
    /// Border-box sizes are unscaled. Small resize noise must not restart layout.
    pub fn measure(&mut self, id: NodeId, size: Size) -> bool {
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return false;
        }
        self.pending.shift_remove(&id);
        let changed = self.measured.get(&id).is_none_or(|old| {
            (old.width - size.width).abs() > 8.0 || (old.height - size.height).abs() > 8.0
        });
        if changed {
            self.measured.insert(id, size);
        }
        changed
    }

    pub fn merge_positions<N, E>(
        &mut self,
        model: &GraphModel<N, E>,
        positions: IndexMap<NodeId, Point>,
    ) {
        let mut next = IndexMap::new();
        for node in model.nodes() {
            let position = if node.pinned {
                node.position
                    .or_else(|| self.positions.get(&node.id).copied())
            } else if self.moved.contains(&node.id) {
                self.positions.get(&node.id).copied()
            } else {
                positions.get(&node.id).copied().or(node.position)
            };
            if let Some(position) = position {
                next.insert(node.id.clone(), position);
            }
        }
        self.positions = next;
        self.selection.retain(|id| model.node(id).is_some());
        self.measured.retain(|id, _| model.node(id).is_some());
        self.moved.retain(|id| model.node(id).is_some());
        self.stable.retain(|id| model.node(id).is_some());
    }
}

pub fn needs_recull(culled: Rect, visible: Rect, last_zoom: f64, zoom: f64) -> bool {
    !culled.contains(visible.origin)
        || !culled.contains(Point {
            x: visible.origin.x + visible.size.width,
            y: visible.origin.y + visible.size.height,
        })
        || last_zoom <= 0.0
        || ((zoom / last_zoom) - 1.0).abs() > 0.15
}

pub fn detail_for_zoom(zoom: f64, full: f64, compact: f64) -> NodeDetail {
    if zoom >= full {
        NodeDetail::Full
    } else if zoom >= compact {
        NodeDetail::Compact
    } else {
        NodeDetail::Minimal
    }
}

pub fn visible_nodes(
    rects: &IndexMap<NodeId, Rect>,
    visible: Rect,
    selected: &IndexSet<NodeId>,
    dragging: Option<&NodeId>,
) -> IndexSet<NodeId> {
    rects
        .iter()
        .filter(|(id, rect)| {
            rect.intersects(visible) || selected.contains(*id) || dragging == Some(*id)
        })
        .map(|(id, _)| id.clone())
        .collect()
}

pub fn edge_visible(source: &NodeId, target: &NodeId, visible: &IndexSet<NodeId>) -> bool {
    visible.contains(source) || visible.contains(target)
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectionEvent {
    Click { node: NodeId, shift: bool },
    Clear,
}

pub fn reduce_selection(current: &IndexSet<NodeId>, event: SelectionEvent) -> IndexSet<NodeId> {
    match event {
        SelectionEvent::Clear => IndexSet::new(),
        SelectionEvent::Click { node, shift: false } => IndexSet::from([node]),
        SelectionEvent::Click { node, shift: true } => {
            let mut next = current.clone();
            if !next.shift_remove(&node) {
                next.insert(node);
            }
            next
        }
    }
}

/// A structural snapshot excludes content and positions: dragging does not lay out.
pub fn layout_signature<N, E>(model: &GraphModel<N, E>) -> Vec<String> {
    let mut signature: Vec<_> = model
        .nodes()
        .map(|node| {
            format!(
                "n:{}:{:?}:{:?}:{}:{}",
                node.id,
                node.layout_parent,
                node.order_key,
                node.size_hint.width,
                node.size_hint.height
            )
        })
        .chain(
            model
                .edges()
                .map(|edge| format!("e:{}:{}:{}", edge.id, edge.source, edge.target)),
        )
        .collect();
    signature.sort();
    signature
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64) -> Rect {
        Rect {
            origin: Point { x, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 100.0,
            },
        }
    }
    #[test]
    fn recull_has_margin_and_zoom_hysteresis() {
        let c = rect(0.0).inflate(100.0);
        assert!(!needs_recull(c, rect(20.0), 1.0, 1.1));
        assert!(needs_recull(c, rect(201.0), 1.0, 1.0));
        assert!(needs_recull(c, rect(0.0), 1.0, 1.2));
    }
    #[test]
    fn culling_keeps_selected_and_incident_edges() {
        let a = NodeId::from("a");
        let b = NodeId::from("b");
        let rects = IndexMap::from([(a.clone(), rect(0.0)), (b.clone(), rect(999.0))]);
        let v = visible_nodes(&rects, rect(0.0), &IndexSet::new(), None);
        assert_eq!(v.len(), 1);
        assert!(edge_visible(&a, &b, &v));
        assert_eq!(
            visible_nodes(&rects, rect(0.0), &IndexSet::from([b]), None).len(),
            2
        );
    }
    #[test]
    fn detail_thresholds() {
        assert_eq!(detail_for_zoom(0.6, 0.6, 0.3), NodeDetail::Full);
        assert_eq!(detail_for_zoom(0.3, 0.6, 0.3), NodeDetail::Compact);
        assert_eq!(detail_for_zoom(0.29, 0.6, 0.3), NodeDetail::Minimal);
    }
    #[test]
    fn selection_shift_toggles_and_escape_clears() {
        let a = NodeId::from("a");
        let b = NodeId::from("b");
        let selection = reduce_selection(
            &IndexSet::from([a.clone()]),
            SelectionEvent::Click {
                node: b.clone(),
                shift: true,
            },
        );
        assert_eq!(selection.len(), 2);
        assert_eq!(
            reduce_selection(
                &selection,
                SelectionEvent::Click {
                    node: a,
                    shift: true
                }
            ),
            IndexSet::from([b])
        );
        assert!(reduce_selection(&selection, SelectionEvent::Clear).is_empty());
    }
    #[test]
    fn measurement_filters_noise() {
        let mut state = CanvasState::default();
        assert!(state.measure(
            "a".into(),
            Size {
                width: 100.0,
                height: 50.0
            }
        ));
        assert!(!state.measure(
            "a".into(),
            Size {
                width: 107.0,
                height: 51.0
            }
        ));
        assert!(state.measure(
            "a".into(),
            Size {
                width: 109.0,
                height: 51.0
            }
        ));
    }
    #[test]
    fn drag_effects_keep_position_and_mark_moved() {
        use crate::interaction::GestureEffect;
        let mut state = CanvasState::default();
        let id = NodeId::from("a");
        let p = Point { x: 42.0, y: 9.0 };
        state.apply_effect(&GestureEffect::NodeDragStart(id.clone()));
        assert_eq!(state.dragging, Some(id.clone()));
        state.apply_effect(&GestureEffect::NodeDragMove {
            node: id.clone(),
            position: p,
        });
        state.apply_effect(&GestureEffect::NodeDragEnd {
            node: id.clone(),
            position: p,
        });
        assert!(state.dragging.is_none());
        assert!(state.moved.contains(&id));
        assert_eq!(state.positions[&id], p);
    }
    #[test]
    fn pinned_and_user_moved_positions_win_and_removal_prunes() {
        let mut model = GraphModel::<(), ()>::default();
        let mut pinned = crate::GraphNode::new("p", ());
        pinned.position = Some(Point { x: 7.0, y: 8.0 });
        pinned.pinned = true;
        model.insert_node(pinned).unwrap();
        model.insert_node(crate::GraphNode::new("u", ())).unwrap();
        let mut state = CanvasState::default();
        state
            .positions
            .insert("u".into(), Point { x: 11.0, y: 12.0 });
        state.moved.insert("u".into());
        state.merge_positions(
            &model,
            IndexMap::from([
                ("p".into(), Point::default()),
                ("u".into(), Point::default()),
            ]),
        );
        assert_eq!(
            state.positions[&NodeId::from("p")],
            Point { x: 7.0, y: 8.0 }
        );
        assert_eq!(
            state.positions[&NodeId::from("u")],
            Point { x: 11.0, y: 12.0 }
        );
        model.remove_node(&"u".into());
        state.merge_positions(&model, IndexMap::new());
        assert!(!state.positions.contains_key(&NodeId::from("u")));
    }
    #[test]
    fn layout_signature_ignores_drag_and_payload_but_detects_structure() {
        let mut model = GraphModel::<String, ()>::default();
        model
            .insert_node(crate::GraphNode::new("a", "one".into()))
            .unwrap();
        let before = layout_signature(&model);
        model.node_mut(&"a".into()).unwrap().data = "two".into();
        model
            .set_position(&"a".into(), Point { x: 10.0, y: 12.0 })
            .unwrap();
        assert_eq!(layout_signature(&model), before);
        model
            .insert_node(crate::GraphNode::new("b", "three".into()))
            .unwrap();
        assert_ne!(layout_signature(&model), before);
    }
}
