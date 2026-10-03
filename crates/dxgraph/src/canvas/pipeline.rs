use super::{
    NodeDetail,
    state::{CanvasState, SelectionEvent, reduce_selection},
};
use crate::interaction::GestureEffect;
use crate::layout::{LayoutConfig, LayoutEdge, LayoutInput, LayoutNode, run_layout};
use crate::{GraphModel, NodeId, Rect, Size};
use indexmap::IndexMap;
use std::rc::Rc;

pub(super) fn rects<N, E>(model: &GraphModel<N, E>, state: &CanvasState) -> IndexMap<NodeId, Rect> {
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

pub(super) fn calculate_layout<N, E>(
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

/// Events affecting world geometry. Viewport zoom never changes layout sizes.
pub(super) enum CanvasInput {
    Start,
    ModelChanged,
    LayoutChanged(LayoutConfig, u64),
    ResetPositions(u64),
    Measured {
        id: NodeId,
        size: Size,
        detail: NodeDetail,
    },
    FlushMeasurements(u64),
    MeasureTimeout(u64),
    ContainerResized(Size),
    Gesture(GestureEffect),
    Selection(SelectionEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CanvasEffect {
    RectsChanged,
    ContainerChanged,
    FitView,
    ScheduleTimeout(u64),
    ScheduleMeasurements(u64),
}

/// Pure owner of measurement, layout, geometry revisions and initial fitting.
/// The UI only applies effects and renders the stored rectangles.
pub(super) struct CanvasPipeline {
    pub state: CanvasState,
    pub rects: Rc<IndexMap<NodeId, Rect>>,
    pub revision: u64,
    pub container: Option<Size>,
    layout: LayoutConfig,
    layout_revision: u64,
    reset_revision: u64,
    signature: Vec<String>,
    batch: u64,
    flush: u64,
    flush_scheduled: bool,
    dirty: bool,
    preserve: bool,
    measured_once: bool,
    fitted: bool,
    layout_sizes: IndexMap<NodeId, Size>,
}

impl CanvasPipeline {
    pub fn new<N, E>(
        model: &GraphModel<N, E>,
        layout: LayoutConfig,
        layout_revision: u64,
        reset_revision: u64,
    ) -> Self {
        let mut pipeline = Self {
            state: CanvasState::default(),
            rects: Rc::default(),
            revision: 0,
            container: None,
            layout,
            layout_revision,
            reset_revision,
            signature: super::state::layout_signature(model),
            batch: 1,
            flush: 0,
            flush_scheduled: false,
            dirty: false,
            preserve: false,
            measured_once: false,
            fitted: false,
            layout_sizes: IndexMap::new(),
        };
        pipeline.state.pending = model.nodes().map(|node| node.id.clone()).collect();
        pipeline.layout(model, false);
        pipeline.refresh_rects(model, &mut Vec::new());
        pipeline
    }

    pub fn reduce<N, E>(
        &mut self,
        model: &GraphModel<N, E>,
        input: CanvasInput,
    ) -> Vec<CanvasEffect> {
        let mut effects = Vec::new();
        match input {
            CanvasInput::Start => {
                effects.push(CanvasEffect::RectsChanged);
                if !self.state.pending.is_empty() {
                    effects.push(CanvasEffect::ScheduleTimeout(self.batch));
                }
            }
            CanvasInput::ModelChanged => {
                let signature = super::state::layout_signature(model);
                if self.signature != signature {
                    self.signature = signature;
                    self.state.stable = self.state.positions.keys().cloned().collect();
                    self.restart_measurement(model, true, &mut effects);
                }
                self.refresh_rects(model, &mut effects);
            }
            CanvasInput::LayoutChanged(layout, revision) => {
                if self.layout != layout || self.layout_revision != revision {
                    self.layout = layout;
                    self.layout_revision = revision;
                    self.state.stable.clear();
                    self.restart_measurement(model, false, &mut effects);
                }
            }
            CanvasInput::ResetPositions(revision) => {
                if self.reset_revision != revision {
                    self.reset_revision = revision;
                    self.state.moved.clear();
                    self.state.stable.clear();
                    self.restart_measurement(model, false, &mut effects);
                }
            }
            CanvasInput::Measured { id, size, detail } => {
                // Measurements belong to Full detail, including the initial hidden pass.
                if detail != NodeDetail::Full || model.node(&id).is_none() || !valid_size(size) {
                    return effects;
                }
                let was_pending = self.state.pending.contains(&id);
                let changed = self.state.measured.get(&id) != Some(&size);
                self.state.measure(id.clone(), size);
                if changed {
                    let significant = self.layout_sizes.get(&id).is_none_or(|old| {
                        (old.width - size.width).abs() > 8.0
                            || (old.height - size.height).abs() > 8.0
                    });
                    self.dirty |= significant || was_pending;
                    self.refresh_rects(model, &mut effects);
                }
                if self.state.pending.is_empty() && (self.dirty || !self.measured_once) {
                    self.schedule_flush(&mut effects);
                }
            }
            CanvasInput::FlushMeasurements(flush) => {
                if flush == self.flush && self.flush_scheduled {
                    self.flush_scheduled = false;
                    if self.state.pending.is_empty() {
                        self.finish_measurement(model, &mut effects);
                    }
                }
            }
            CanvasInput::MeasureTimeout(batch) => {
                if batch == self.batch && !self.state.pending.is_empty() {
                    self.state.pending.clear();
                    self.finish_measurement(model, &mut effects);
                }
            }
            CanvasInput::ContainerResized(size) => {
                if valid_size(size) && self.container != Some(size) {
                    self.container = Some(size);
                    effects.push(CanvasEffect::ContainerChanged);
                }
            }
            CanvasInput::Gesture(effect) => {
                self.state.apply_effect(&effect);
                if matches!(
                    effect,
                    GestureEffect::NodeDragMove { .. } | GestureEffect::NodeDragEnd { .. }
                ) {
                    self.refresh_rects(model, &mut effects);
                }
            }
            CanvasInput::Selection(event) => {
                self.state.selection = reduce_selection(&self.state.selection, event);
            }
        }
        self.maybe_fit(&mut effects);
        effects
    }

    fn restart_measurement<N, E>(
        &mut self,
        model: &GraphModel<N, E>,
        preserve: bool,
        effects: &mut Vec<CanvasEffect>,
    ) {
        self.batch += 1;
        self.flush += 1;
        self.flush_scheduled = false;
        self.dirty = false;
        self.preserve = preserve && self.measured_once;
        self.state.pending = model
            .nodes()
            .filter(|node| !self.state.measured.contains_key(&node.id))
            .map(|node| node.id.clone())
            .collect();
        self.layout(model, preserve);
        self.refresh_rects(model, effects);
        if !self.state.pending.is_empty() {
            effects.push(CanvasEffect::ScheduleTimeout(self.batch));
        }
    }

    fn schedule_flush(&mut self, effects: &mut Vec<CanvasEffect>) {
        if !self.flush_scheduled {
            self.flush += 1;
            self.flush_scheduled = true;
            effects.push(CanvasEffect::ScheduleMeasurements(self.flush));
        }
    }

    fn finish_measurement<N, E>(
        &mut self,
        model: &GraphModel<N, E>,
        effects: &mut Vec<CanvasEffect>,
    ) {
        self.layout(model, self.preserve);
        self.dirty = false;
        self.measured_once = true;
        self.preserve = false;
        self.state.stable = self.state.positions.keys().cloned().collect();
        self.refresh_rects(model, effects);
    }

    fn layout<N, E>(&mut self, model: &GraphModel<N, E>, preserve: bool) {
        calculate_layout(model, &mut self.state, &self.layout, preserve);
        self.layout_sizes = model
            .nodes()
            .map(|node| {
                (
                    node.id.clone(),
                    self.state
                        .measured
                        .get(&node.id)
                        .copied()
                        .unwrap_or(node.size_hint),
                )
            })
            .collect();
    }

    fn refresh_rects<N, E>(&mut self, model: &GraphModel<N, E>, effects: &mut Vec<CanvasEffect>) {
        let rects = rects(model, &self.state);
        if *self.rects != rects {
            self.rects = Rc::new(rects);
            self.revision += 1;
            effects.push(CanvasEffect::RectsChanged);
        }
    }

    fn maybe_fit(&mut self, effects: &mut Vec<CanvasEffect>) {
        if !self.fitted
            && self.container.is_some()
            && self.state.pending.is_empty()
            && !self.dirty
            && !self.flush_scheduled
            && !self.rects.is_empty()
        {
            self.fitted = true;
            effects.push(CanvasEffect::FitView);
        }
    }
}

fn valid_size(size: Size) -> bool {
    size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphNode, LayoutAlgorithm, LayoutOutput, Point};
    use std::{cell::Cell, rc::Rc};

    struct CountingLayout(Rc<Cell<usize>>);
    impl LayoutAlgorithm for CountingLayout {
        fn layout(&self, input: &LayoutInput) -> LayoutOutput {
            self.0.set(self.0.get() + 1);
            LayoutOutput {
                positions: input
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, node)| {
                        (
                            node.id.clone(),
                            node.fixed.unwrap_or(Point::new(index as f64 * 500.0, 20.0)),
                        )
                    })
                    .collect(),
            }
        }
    }

    fn fixture() -> (GraphModel<(), ()>, CanvasPipeline, Rc<Cell<usize>>) {
        let mut model = GraphModel::default();
        for id in ["a", "b"] {
            model.insert_node(GraphNode::new(id, ())).unwrap();
        }
        let count = Rc::new(Cell::new(0));
        let pipeline = CanvasPipeline::new(
            &model,
            LayoutConfig::Custom(Rc::new(CountingLayout(count.clone()))),
            0,
            0,
        );
        (model, pipeline, count)
    }

    fn measure(
        pipeline: &mut CanvasPipeline,
        model: &GraphModel<(), ()>,
        id: &str,
        size: Size,
        detail: NodeDetail,
    ) -> Vec<CanvasEffect> {
        pipeline.reduce(
            model,
            CanvasInput::Measured {
                id: id.into(),
                size,
                detail,
            },
        )
    }

    fn flush(pipeline: &mut CanvasPipeline, model: &GraphModel<(), ()>) -> Vec<CanvasEffect> {
        pipeline.reduce(model, CanvasInput::FlushMeasurements(pipeline.flush))
    }

    fn settle(pipeline: &mut CanvasPipeline, model: &GraphModel<(), ()>) {
        measure(
            pipeline,
            model,
            "a",
            Size::new(100.0, 60.0),
            NodeDetail::Full,
        );
        measure(
            pipeline,
            model,
            "b",
            Size::new(100.0, 60.0),
            NodeDetail::Full,
        );
        flush(pipeline, model);
    }

    #[test]
    fn lower_detail_measurements_never_change_layout_sizes_or_positions() {
        let (model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        let before = pipeline.rects.clone();
        for detail in [NodeDetail::Compact, NodeDetail::Minimal] {
            let effects = measure(&mut pipeline, &model, "a", Size::new(70.0, 20.0), detail);
            assert!(effects.is_empty());
        }
        // Returning to Full with the same content also leaves layout untouched.
        assert!(
            measure(
                &mut pipeline,
                &model,
                "a",
                Size::new(100.0, 60.0),
                NodeDetail::Full
            )
            .is_empty()
        );
        assert_eq!(pipeline.rects, before);
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn measurement_bursts_schedule_one_layout_using_all_received_sizes() {
        let (model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        let first = measure(
            &mut pipeline,
            &model,
            "a",
            Size::new(140.0, 80.0),
            NodeDetail::Full,
        );
        let second = measure(
            &mut pipeline,
            &model,
            "b",
            Size::new(160.0, 90.0),
            NodeDetail::Full,
        );
        assert_eq!(
            first
                .iter()
                .filter(|effect| matches!(effect, CanvasEffect::ScheduleMeasurements(_)))
                .count(),
            1
        );
        assert!(
            !second
                .iter()
                .any(|effect| matches!(effect, CanvasEffect::ScheduleMeasurements(_)))
        );
        assert_eq!(count.get(), 2, "resize callbacks must not run the engine");
        flush(&mut pipeline, &model);
        assert_eq!(count.get(), 3);
        assert_eq!(
            pipeline.rects[&NodeId::from("a")].size,
            Size::new(140.0, 80.0)
        );
        assert_eq!(
            pipeline.rects[&NodeId::from("b")].size,
            Size::new(160.0, 90.0)
        );
        flush(&mut pipeline, &model);
        assert_eq!(count.get(), 3, "a duplicate tick must be harmless");
    }

    #[test]
    fn small_size_changes_update_geometry_and_accumulate_against_last_layout() {
        let (model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        let revision = pipeline.revision;
        let effects = measure(
            &mut pipeline,
            &model,
            "a",
            Size::new(107.0, 60.0),
            NodeDetail::Full,
        );
        assert_eq!(effects, vec![CanvasEffect::RectsChanged]);
        assert_eq!(pipeline.rects[&NodeId::from("a")].size.width, 107.0);
        assert!(pipeline.revision > revision);
        let effects = measure(
            &mut pipeline,
            &model,
            "a",
            Size::new(109.0, 60.0),
            NodeDetail::Full,
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, CanvasEffect::ScheduleMeasurements(_)))
        );
        flush(&mut pipeline, &model);
        assert_eq!(count.get(), 3);
    }

    #[test]
    fn initial_fit_waits_for_container_and_coalesced_measurements() {
        for container_first in [true, false] {
            let (model, mut pipeline, count) = fixture();
            if container_first {
                assert!(
                    !pipeline
                        .reduce(
                            &model,
                            CanvasInput::ContainerResized(Size::new(800.0, 600.0))
                        )
                        .contains(&CanvasEffect::FitView)
                );
            }
            measure(
                &mut pipeline,
                &model,
                "a",
                Size::new(100.0, 60.0),
                NodeDetail::Full,
            );
            let effects = measure(
                &mut pipeline,
                &model,
                "b",
                Size::new(100.0, 60.0),
                NodeDetail::Full,
            );
            assert!(!effects.contains(&CanvasEffect::FitView));
            let effects = flush(&mut pipeline, &model);
            assert_eq!(effects.contains(&CanvasEffect::FitView), container_first);
            let effects = pipeline.reduce(
                &model,
                CanvasInput::ContainerResized(Size::new(900.0, 700.0)),
            );
            assert_eq!(effects.contains(&CanvasEffect::FitView), !container_first);
            assert_eq!(count.get(), 2, "container resizing must never run layout");
            assert!(
                !pipeline
                    .reduce(
                        &model,
                        CanvasInput::ContainerResized(Size::new(600.0, 500.0))
                    )
                    .contains(&CanvasEffect::FitView)
            );
        }
    }

    #[test]
    fn stale_measurement_timeout_cannot_complete_a_new_model_batch() {
        let (mut model, mut pipeline, count) = fixture();
        let first_batch = pipeline.batch;
        model.insert_node(GraphNode::new("c", ())).unwrap();
        pipeline.reduce(&model, CanvasInput::ModelChanged);
        assert!(
            pipeline
                .reduce(&model, CanvasInput::MeasureTimeout(first_batch))
                .is_empty()
        );
        assert_eq!(pipeline.state.pending.len(), 3);
        let current_batch = pipeline.batch;
        pipeline.reduce(&model, CanvasInput::MeasureTimeout(current_batch));
        assert!(pipeline.state.pending.is_empty());
        assert_eq!(pipeline.rects.len(), 3);
        assert_eq!(count.get(), 3);
    }

    #[test]
    fn incremental_measurement_retains_dragged_and_existing_positions() {
        let (mut model, mut pipeline, _count) = fixture();
        settle(&mut pipeline, &model);
        let point = Point::new(45.0, 90.0);
        pipeline.reduce(
            &model,
            CanvasInput::Gesture(GestureEffect::NodeDragEnd {
                node: "a".into(),
                position: point,
            }),
        );
        let old_b = pipeline.rects[&NodeId::from("b")].origin;
        model.insert_node(GraphNode::new("c", ())).unwrap();
        pipeline.reduce(&model, CanvasInput::ModelChanged);
        measure(
            &mut pipeline,
            &model,
            "c",
            Size::new(130.0, 80.0),
            NodeDetail::Full,
        );
        flush(&mut pipeline, &model);
        assert_eq!(pipeline.rects[&NodeId::from("a")].origin, point);
        assert_eq!(pipeline.rects[&NodeId::from("b")].origin, old_b);
    }

    #[test]
    fn custom_engine_identity_and_controller_revision_request_layout() {
        let (model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        let same_engine = pipeline.layout.clone();
        pipeline.reduce(&model, CanvasInput::LayoutChanged(same_engine.clone(), 0));
        assert_eq!(count.get(), 2);
        pipeline.reduce(&model, CanvasInput::LayoutChanged(same_engine, 1));
        assert_eq!(count.get(), 3);
        let other_count = Rc::new(Cell::new(0));
        pipeline.reduce(
            &model,
            CanvasInput::LayoutChanged(
                LayoutConfig::Custom(Rc::new(CountingLayout(other_count.clone()))),
                1,
            ),
        );
        assert_eq!(other_count.get(), 1);
    }

    #[test]
    fn remount_starts_with_current_controller_revisions() {
        let (model, _, count) = fixture();
        let layout = LayoutConfig::Custom(Rc::new(CountingLayout(count.clone())));
        let mut pipeline = CanvasPipeline::new(&model, layout.clone(), 7, 3);
        let before = count.get();
        pipeline.reduce(&model, CanvasInput::Start);
        pipeline.reduce(&model, CanvasInput::LayoutChanged(layout, 7));
        pipeline.reduce(&model, CanvasInput::ResetPositions(3));
        assert_eq!(count.get(), before);
    }

    #[test]
    fn container_resize_preserves_shared_rects_and_emits_only_container_change() {
        let (model, mut pipeline, _) = fixture();
        let rects = pipeline.rects.clone();
        let revision = pipeline.revision;
        let effects = pipeline.reduce(
            &model,
            CanvasInput::ContainerResized(Size::new(600.0, 400.0)),
        );
        assert_eq!(effects, vec![CanvasEffect::ContainerChanged]);
        assert!(Rc::ptr_eq(&rects, &pipeline.rects));
        assert_eq!(revision, pipeline.revision);
        assert!(
            pipeline
                .reduce(
                    &model,
                    CanvasInput::ContainerResized(Size::new(600.0, 400.0))
                )
                .is_empty()
        );
    }

    #[test]
    fn resetting_positions_releases_dragged_nodes_for_automatic_layout() {
        let (model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        let original = pipeline.state.positions[&NodeId::from("a")];
        pipeline.reduce(
            &model,
            CanvasInput::Gesture(GestureEffect::NodeDragEnd {
                node: "a".into(),
                position: Point::new(100.0, 200.0),
            }),
        );
        let layout = pipeline.layout.clone();
        pipeline.reduce(&model, CanvasInput::LayoutChanged(layout, 1));
        assert_eq!(
            pipeline.state.positions[&NodeId::from("a")],
            Point::new(100.0, 200.0)
        );
        pipeline.reduce(&model, CanvasInput::ResetPositions(1));
        assert!(pipeline.state.moved.is_empty());
        assert!(pipeline.state.stable.is_empty());
        assert_eq!(pipeline.state.positions[&NodeId::from("a")], original);
        let before = count.get();
        assert!(
            pipeline
                .reduce(&model, CanvasInput::ResetPositions(1))
                .is_empty()
        );
        assert_eq!(count.get(), before);
    }

    #[test]
    fn reset_and_relayout_keep_explicit_model_pins() {
        let mut model = GraphModel::<(), ()>::default();
        let mut pinned = GraphNode::new("a", ());
        let pin = Point::new(700.0, 900.0);
        pinned.position = Some(pin);
        pinned.pinned = true;
        model.insert_node(pinned).unwrap();
        model.insert_node(GraphNode::new("b", ())).unwrap();
        let mut pipeline = CanvasPipeline::new(&model, LayoutConfig::default(), 0, 0);
        settle(&mut pipeline, &model);
        assert_eq!(pipeline.state.positions[&NodeId::from("a")], pin);
        let original = pipeline.state.positions[&NodeId::from("b")];
        let dragged = Point::new(100.0, 200.0);
        pipeline.reduce(
            &model,
            CanvasInput::Gesture(GestureEffect::NodeDragEnd {
                node: "b".into(),
                position: dragged,
            }),
        );
        pipeline.reduce(
            &model,
            CanvasInput::LayoutChanged(pipeline.layout.clone(), 1),
        );
        assert_eq!(pipeline.state.positions[&NodeId::from("a")], pin);
        assert_eq!(pipeline.state.positions[&NodeId::from("b")], dragged);
        pipeline.reduce(&model, CanvasInput::ResetPositions(1));
        assert!(pipeline.state.moved.is_empty());
        assert_eq!(pipeline.state.positions[&NodeId::from("a")], pin);
        assert_eq!(pipeline.state.positions[&NodeId::from("b")], original);
        assert!(model.node(&"a".into()).unwrap().pinned);
    }
    #[test]
    fn invalid_measurements_leave_initial_batch_pending() {
        let (model, mut pipeline, count) = fixture();
        for size in [
            Size::new(f64::NAN, 10.0),
            Size::new(0.0, 10.0),
            Size::new(10.0, -1.0),
        ] {
            assert!(measure(&mut pipeline, &model, "a", size, NodeDetail::Full).is_empty());
        }
        assert_eq!(pipeline.state.pending.len(), 2);
        assert_eq!(count.get(), 1);
    }
    #[test]
    fn stale_flush_cannot_finish_measurement_after_model_change() {
        let (mut model, mut pipeline, count) = fixture();
        settle(&mut pipeline, &model);
        measure(
            &mut pipeline,
            &model,
            "a",
            Size::new(150.0, 90.0),
            NodeDetail::Full,
        );
        let stale_flush = pipeline.flush;
        model.insert_node(GraphNode::new("c", ())).unwrap();
        pipeline.reduce(&model, CanvasInput::ModelChanged);
        let layouts = count.get();
        assert!(
            pipeline
                .reduce(&model, CanvasInput::FlushMeasurements(stale_flush))
                .is_empty()
        );
        assert!(pipeline.state.pending.contains(&NodeId::from("c")));
        assert_eq!(count.get(), layouts);
        measure(
            &mut pipeline,
            &model,
            "c",
            Size::new(140.0, 80.0),
            NodeDetail::Full,
        );
        flush(&mut pipeline, &model);
        assert_eq!(count.get(), layouts + 1);
    }
}
