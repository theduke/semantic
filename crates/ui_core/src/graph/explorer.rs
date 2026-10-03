use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::EntityTarget;
use dxgraph::{EdgeId, EdgePathStyle, EdgeStyle, GraphEdge, GraphModel, GraphNode, NodeId, Size};
use semantic_data::{
    attr::ATTR_PARENT,
    value::{Object, Value},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphMode {
    #[default]
    Hierarchy,
    Relations,
    Both,
}
impl GraphMode {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("relations") => Self::Relations,
            Some("both") => Self::Both,
            _ => Self::Hierarchy,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hierarchy => "hierarchy",
            Self::Relations => "relations",
            Self::Both => "both",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplorerLimits {
    pub max_nodes: usize,
    pub fan_out: usize,
}
impl Default for ExplorerLimits {
    fn default() -> Self {
        Self {
            max_nodes: 300,
            fan_out: 50,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntityNodeData {
    Entity {
        target: EntityTarget,
        object: Option<Object>,
        loading: bool,
        expanded: bool,
    },
    Overflow {
        parent: NodeId,
    },
}
impl EntityNodeData {
    pub fn target(&self) -> Option<&EntityTarget> {
        match self {
            Self::Entity { target, .. } => Some(target),
            _ => None,
        }
    }
    pub fn object(&self) -> Option<&Object> {
        match self {
            Self::Entity { object, .. } => object.as_ref(),
            _ => None,
        }
    }
    pub fn id(&self) -> NodeId {
        match self {
            Self::Entity { target, .. } => node_id(target),
            Self::Overflow { parent } => NodeId(format!("overflow:{parent}")),
        }
    }
    pub fn loading(&self) -> bool {
        matches!(self, Self::Entity { loading: true, .. })
    }
    pub fn expanded(&self) -> bool {
        matches!(self, Self::Entity { expanded: true, .. })
    }
    fn set_state(&mut self, is_loading: bool, is_expanded: bool) {
        if let Self::Entity {
            loading, expanded, ..
        } = self
        {
            *loading = is_loading;
            *expanded = is_expanded;
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct EntityEdgeData {
    pub kind: EntityEdgeKind,
}
#[derive(Clone, Debug, PartialEq)]
pub enum EntityEdgeKind {
    Parent,
    Relation { relation_id: String, label: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ExpansionRequest {
    pub node: NodeId,
    pub target: String,
    pub mode: GraphMode,
    pub limit: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ExpansionEdge {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EntityEdgeKind,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpansionResult {
    pub nodes: Vec<EntityNodeData>,
    pub edges: Vec<ExpansionEdge>,
}

pub fn node_id(target: &EntityTarget) -> NodeId {
    let collection = target.collection_or_default();
    NodeId(format!(
        "entity:{}:{collection}{}",
        collection.len(),
        target.id
    ))
}

fn entity_edge_id(kind: &EntityEdgeKind, source: &NodeId, target: &NodeId) -> EdgeId {
    match kind {
        EntityEdgeKind::Parent => EdgeId(format!("parent:{target}->{source}")),
        EntityEdgeKind::Relation { relation_id, .. } => {
            EdgeId(format!("rel:{relation_id}:{source}->{target}"))
        }
    }
}

pub fn entity_data(id: impl Into<String>, object: Option<Object>) -> EntityNodeData {
    EntityNodeData::Entity {
        target: EntityTarget::default_collection(id),
        object,
        loading: false,
        expanded: false,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityGraphExplorer {
    model: GraphModel<EntityNodeData, EntityEdgeData>,
    mode: GraphMode,
    limits: ExplorerLimits,
    root: EntityTarget,
    // Expansion provenance, separate from semantic edge direction, drives collapse.
    expansion_nodes: BTreeMap<NodeId, BTreeSet<NodeId>>,
    expansion_edges: BTreeMap<NodeId, BTreeSet<EdgeId>>,
}

impl EntityGraphExplorer {
    pub fn new(root: impl Into<String>, mode: GraphMode, limits: ExplorerLimits) -> Self {
        let root = EntityTarget::default_collection(root);
        let mut this = Self {
            model: GraphModel::default(),
            mode,
            limits,
            root: root.clone(),
            expansion_nodes: BTreeMap::new(),
            expansion_edges: BTreeMap::new(),
        };
        this.model
            .upsert_node(make_node(entity_data(root.id, None), None));
        this
    }
    pub fn model(&self) -> &GraphModel<EntityNodeData, EntityEdgeData> {
        &self.model
    }
    pub fn root(&self) -> &EntityTarget {
        &self.root
    }
    pub fn mode(&self) -> GraphMode {
        self.mode
    }
    pub fn limits(&self) -> ExplorerLimits {
        self.limits
    }
    fn set_state(&mut self, id: &NodeId, loading: bool, expanded: bool) {
        if let Some(data) = self.model.node_data_mut(id) {
            data.set_state(loading, expanded);
        }
    }
    pub fn is_expanded(&self, id: &NodeId) -> bool {
        self.model.node(id).is_some_and(|node| node.data.expanded())
    }
    pub fn is_loading(&self, id: &NodeId) -> bool {
        self.model.node(id).is_some_and(|node| node.data.loading())
    }
    pub fn set_object(&mut self, target: &EntityTarget, object: Object) {
        if let Some(data) = self.model.node_data_mut(&node_id(target))
            && let EntityNodeData::Entity {
                object: current, ..
            } = data
        {
            *current = Some(object);
        }
    }
    pub fn begin_expand(&mut self, id: &NodeId) -> Option<ExpansionRequest> {
        if self.is_expanded(id)
            || self.is_loading(id)
            || self.model.nodes().count() >= self.limits.max_nodes
        {
            return None;
        }
        let node = self.model.node(id)?;
        let request = ExpansionRequest {
            node: id.clone(),
            target: node.data.target()?.id.clone(),
            mode: self.mode,
            limit: self.limits.fan_out.saturating_add(1),
        };
        self.set_state(id, true, false);
        Some(request)
    }
    pub fn fail_expansion(&mut self, id: &NodeId) {
        self.set_state(id, false, self.is_expanded(id));
    }
    pub fn apply_expansion(&mut self, id: &NodeId, result: ExpansionResult) {
        self.set_state(id, false, self.is_expanded(id));
        if self.model.node(id).is_none() {
            return;
        }
        let current = self.model.nodes().count();
        let mut new_count = 0;
        let unique_new = result
            .nodes
            .iter()
            .map(EntityNodeData::id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|id| self.model.node(id).is_none())
            .count();
        let available = self.limits.max_nodes.saturating_sub(current);
        let needs_overflow = unique_new > available || unique_new > self.limits.fan_out;
        let budget = available
            .saturating_sub(usize::from(needs_overflow))
            .min(self.limits.fan_out);
        let mut owned = BTreeSet::new();
        let mut omitted = 0;
        for data in result.nodes {
            let target = data.id();
            if self.model.node(&target).is_none() {
                if new_count >= budget {
                    omitted += 1;
                    continue;
                }
                new_count += 1;
            }
            owned.insert(target.clone());
            if let Some(existing) = self.model.node_data_mut(&target) {
                if data.object().is_some() {
                    let state = (existing.loading(), existing.expanded());
                    *existing = data;
                    existing.set_state(state.0, state.1);
                }
            } else {
                self.model.upsert_node(make_node(data, Some(id.clone())));
            }
        }
        let mut owned_edges = BTreeSet::new();
        for edge in result.edges {
            let source = edge.source;
            let target = edge.target;
            if self.model.node(&source).is_none() || self.model.node(&target).is_none() {
                continue;
            }
            let edge_id = entity_edge_id(&edge.kind, &source, &target);
            let style = match &edge.kind {
                EntityEdgeKind::Parent => EdgeStyle {
                    path: EdgePathStyle::Bezier,
                    ..EdgeStyle::default()
                },
                EntityEdgeKind::Relation { relation_id, .. } => EdgeStyle {
                    path: EdgePathStyle::Bezier,
                    class: Some(format!(
                        "semantic-graph-relation-{}",
                        stable_color(relation_id)
                    )),
                    ..EdgeStyle::default()
                },
            };
            owned_edges.insert(edge_id.clone());
            if !self.model.contains_edge(&edge_id) {
                let _ = self.model.insert_edge(GraphEdge {
                    id: edge_id,
                    source,
                    target,
                    label: match &edge.kind {
                        EntityEdgeKind::Relation { label, .. } => Some(label.clone()),
                        _ => None,
                    },
                    data: EntityEdgeData { kind: edge.kind },
                    style,
                });
            }
        }
        if omitted > 0 && self.model.nodes().count() < self.limits.max_nodes {
            let data = EntityNodeData::Overflow { parent: id.clone() };
            let overflow_id = data.id();
            self.model.upsert_node(make_node(data, Some(id.clone())));
            owned.insert(overflow_id.clone());
            let edge_id = EdgeId(format!("overflow:{id}"));
            owned_edges.insert(edge_id.clone());
            let _ = self.model.insert_edge(GraphEdge {
                id: edge_id,
                source: id.clone(),
                target: overflow_id,
                label: None,
                data: EntityEdgeData {
                    kind: EntityEdgeKind::Parent,
                },
                style: EdgeStyle {
                    dashed: true,
                    ..EdgeStyle::default()
                },
            });
        }
        self.expansion_nodes.insert(id.clone(), owned);
        self.expansion_edges.insert(id.clone(), owned_edges);
        self.set_state(id, false, true);
    }
    pub fn collapse(&mut self, id: &NodeId) {
        self.expansion_nodes.remove(id);
        if let Some(edges) = self.expansion_edges.remove(id) {
            for edge in edges {
                if !self
                    .expansion_edges
                    .values()
                    .any(|owned| owned.contains(&edge))
                {
                    self.model.remove_edge(&edge);
                }
            }
        }
        self.set_state(id, false, false);
        let root = node_id(&self.root);
        let mut retained = BTreeSet::from([root.clone()]);
        let mut pending = VecDeque::from([root.clone()]);
        while let Some(parent) = pending.pop_front() {
            if let Some(children) = self.expansion_nodes.get(&parent) {
                for child in children {
                    if retained.insert(child.clone()) {
                        pending.push_back(child.clone());
                    }
                }
            }
        }
        let removed = self
            .model
            .nodes()
            .filter(|node| !retained.contains(&node.id))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        for id in removed {
            self.model.remove_node(&id);
            self.expansion_nodes.remove(&id);
            self.expansion_edges.remove(&id);
        }
        // A surviving shared node may have had its layout parent removed.
        let parents = self
            .expansion_nodes
            .iter()
            .flat_map(|(parent, children)| {
                children
                    .iter()
                    .map(move |child| (child.clone(), parent.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let repairs = parents.into_iter().filter(|(node, parent)| {
            node != &root
                && self.model.node(node).is_some_and(|item| item.layout_parent.is_none())
                // Ancestor provenance points upward; never turn it into a layout cycle.
                && self.model.node(parent).is_some_and(|item| item.layout_parent.as_ref() != Some(node))
        }).collect::<Vec<_>>();
        for (node, parent) in repairs {
            let _ = self.model.set_layout_parent(&node, Some(parent));
        }
    }
    /// Parent references identify entities in the default collection.
    pub fn ancestors_request(&self, id: &NodeId) -> Option<EntityTarget> {
        let node = self.model.node(id)?;
        let parent = node.data.object()?.get(ATTR_PARENT)?.as_str()?;
        let target = EntityTarget::default_collection(parent);
        (self.model.node(&node_id(&target)).is_none()).then_some(target)
    }
    pub fn apply_ancestor(&mut self, child: &NodeId, data: EntityNodeData) {
        if self.model.nodes().count() >= self.limits.max_nodes {
            return;
        }
        let parent = data.id();
        let Some(child_target) = self
            .model
            .node(child)
            .and_then(|node| node.data.target().cloned())
        else {
            return;
        };
        if let Some(existing) = self.model.node_data_mut(&parent) {
            if data.object().is_some() {
                let state = (existing.loading(), existing.expanded());
                *existing = data;
                existing.set_state(state.0, state.1);
            }
        } else {
            self.model.upsert_node(make_node(data, None));
        }
        let _ = self.model.set_layout_parent(child, Some(parent.clone()));
        self.expansion_nodes
            .entry(child.clone())
            .or_default()
            .insert(parent.clone());
        let edge_id = entity_edge_id(&EntityEdgeKind::Parent, &parent, child);
        self.expansion_edges
            .entry(child.clone())
            .or_default()
            .insert(edge_id.clone());
        let _ = self.model.insert_edge(GraphEdge {
            id: edge_id,
            source: parent,
            target: node_id(&child_target),
            label: None,
            data: EntityEdgeData {
                kind: EntityEdgeKind::Parent,
            },
            style: EdgeStyle::default(),
        });
    }
}

fn make_node(data: EntityNodeData, parent: Option<NodeId>) -> GraphNode<EntityNodeData> {
    GraphNode {
        id: data.id(),
        order_key: data
            .object()
            .and_then(|object| object.get("semantic:title"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        data,
        position: None,
        size_hint: Size {
            width: 220.0,
            height: 88.0,
        },
        pinned: false,
        layout_parent: parent,
    }
}
pub(super) fn stable_color(id: &str) -> u64 {
    id.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
    }) % 8
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: &str) -> EntityTarget {
        EntityTarget::default_collection(id)
    }
    fn id(name: &str) -> NodeId {
        node_id(&target(name))
    }
    fn fixture(nodes: &[&str], source: &str) -> ExpansionResult {
        ExpansionResult {
            nodes: nodes
                .iter()
                .map(|node| entity_data(*node, Some(Object::new())))
                .collect(),
            edges: nodes
                .iter()
                .map(|node| ExpansionEdge {
                    source: id(source),
                    target: id(node),
                    kind: EntityEdgeKind::Parent,
                })
                .collect(),
        }
    }
    #[test]
    fn seed_expand_deduplicate() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Hierarchy, ExplorerLimits::default());
        assert_eq!(graph.model.nodes().count(), 1);
        assert!(graph.begin_expand(&id("root")).is_some());
        assert!(graph.begin_expand(&id("root")).is_none());
        graph.apply_expansion(&id("root"), fixture(&["a", "b", "a"], "root"));
        assert_eq!(graph.model.nodes().count(), 3);
        assert_eq!(graph.model.edges().count(), 2);
        graph.set_object(&target("root"), Object::new());
    }
    #[test]
    fn fanout_and_global_caps_include_overflow() {
        for limits in [
            ExplorerLimits {
                max_nodes: 10,
                fan_out: 2,
            },
            ExplorerLimits {
                max_nodes: 3,
                fan_out: 50,
            },
        ] {
            let mut graph = EntityGraphExplorer::new("root", GraphMode::Both, limits);
            graph.apply_expansion(&id("root"), fixture(&["a", "b", "c", "d"], "root"));
            assert!(graph.model.nodes().count() <= limits.max_nodes);
            assert!(
                graph
                    .model
                    .nodes()
                    .any(|node| matches!(node.data, EntityNodeData::Overflow { .. }))
            );
        }
    }
    #[test]
    fn collapse_keeps_shared_nodes_and_cyclic_relation_nodes() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Relations, ExplorerLimits::default());
        graph.apply_expansion(&id("root"), fixture(&["a", "b"], "root"));
        graph.apply_expansion(&id("a"), fixture(&["shared", "root"], "a"));
        graph.apply_expansion(&id("b"), fixture(&["shared"], "b"));
        graph.collapse(&id("a"));
        assert!(graph.model.node(&id("shared")).is_some());
        assert!(graph.model.node(&id("root")).is_some());
        graph.collapse(&id("b"));
        assert!(graph.model.node(&id("shared")).is_none());
    }
    #[test]
    fn unresolved_endpoints_remain_and_parent_reroots_layout() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Both, ExplorerLimits::default());
        let mut root = Object::new();
        root.insert(ATTR_PARENT, Value::String("parent".into()));
        graph.set_object(&target("root"), root);
        let parent = graph.ancestors_request(&id("root")).unwrap();
        assert_eq!(parent, target("parent"));
        graph.apply_ancestor(&id("root"), entity_data(parent.id, None));
        assert_eq!(
            graph.model.node(&id("root")).unwrap().layout_parent,
            Some(id("parent"))
        );
        assert!(
            graph
                .model
                .node(&id("parent"))
                .unwrap()
                .data
                .object()
                .is_none()
        );
    }

    #[test]
    fn collapse_reparents_shared_survivors_after_parent_removal() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Both, ExplorerLimits::default());
        graph.apply_expansion(&id("root"), fixture(&["a", "b"], "root"));
        graph.apply_expansion(&id("a"), fixture(&["branch"], "a"));
        graph.apply_expansion(&id("branch"), fixture(&["shared"], "branch"));
        graph.apply_expansion(&id("b"), fixture(&["shared"], "b"));
        graph.collapse(&id("a"));
        assert!(graph.model.node(&id("branch")).is_none());
        assert_eq!(
            graph.model.node(&id("shared")).unwrap().layout_parent,
            Some(id("b"))
        );
    }

    #[test]
    fn overflow_ids_cannot_alias_entities_and_have_no_navigation_target() {
        let overflow = EntityNodeData::Overflow { parent: id("root") };
        let real = entity_data(overflow.id().0, None);
        assert_ne!(overflow.id(), real.id());
        assert!(overflow.target().is_none());
        assert_ne!(
            node_id(&EntityTarget::new(Some("a/b".into()), "c")),
            node_id(&EntityTarget::new(Some("a".into()), "b/c"))
        );
    }

    #[test]
    fn expansion_state_only_changes_the_affected_node_payload() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Both, ExplorerLimits::default());
        graph.apply_expansion(&id("root"), fixture(&["a", "b"], "root"));
        let sibling = graph.model().node(&id("b")).unwrap().clone();
        graph.begin_expand(&id("a")).unwrap();
        assert!(graph.model().node(&id("a")).unwrap().data.loading());
        assert_eq!(graph.model().node(&id("b")), Some(&sibling));
        graph.apply_expansion(&id("a"), fixture(&[], "a"));
        assert!(graph.model().node(&id("a")).unwrap().data.expanded());
        assert!(!graph.model().node(&id("a")).unwrap().data.loading());
        graph.collapse(&id("a"));
        assert!(!graph.model().node(&id("a")).unwrap().data.expanded());
        assert_eq!(graph.model().node(&id("b")), Some(&sibling));
    }

    #[test]
    fn replacement_preserves_payload_loading_and_expanded_state() {
        let mut graph =
            EntityGraphExplorer::new("root", GraphMode::Both, ExplorerLimits::default());
        graph.apply_expansion(&id("root"), fixture(&["a", "b"], "root"));
        graph.begin_expand(&id("a")).unwrap();
        graph.apply_expansion(&id("b"), fixture(&["a", "root"], "b"));
        assert!(graph.is_loading(&id("a")));
        assert!(graph.is_expanded(&id("root")));
        graph.fail_expansion(&id("a"));
        assert!(!graph.is_loading(&id("a")));
        assert!(graph.begin_expand(&id("a")).is_some());
        graph.apply_expansion(&id("a"), fixture(&[], "a"));
        assert!(graph.is_expanded(&id("a")));
        assert!(graph.is_expanded(&id("root")));
    }
}
