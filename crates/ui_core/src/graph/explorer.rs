use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::EntityTarget;
use dxgraph::{
    EdgeAnchor, EdgeId, EdgePathStyle, EdgeSide, EdgeStyle, GraphEdge, GraphModel, GraphNode,
    NodeId, Size,
};
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
pub struct EntityNodeData {
    pub target: EntityTarget,
    pub object: Option<Object>,
    pub kind: EntityNodeKind,
}
#[derive(Clone, Debug, PartialEq)]
pub enum EntityNodeKind {
    Entity,
    Unresolved,
    Overflow {
        parent: NodeId,
        remaining_hint: usize,
    },
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
    pub target: EntityTarget,
    pub mode: GraphMode,
    pub limit: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ExpansionEdge {
    pub source: EntityTarget,
    pub target: EntityTarget,
    pub kind: EntityEdgeKind,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpansionResult {
    pub nodes: Vec<EntityNodeData>,
    pub edges: Vec<ExpansionEdge>,
    pub remaining_hint: usize,
}

pub fn node_id(target: &EntityTarget) -> NodeId {
    NodeId(format!("{}/{}", target.collection_or_default(), target.id))
}
pub fn entity_data(target: EntityTarget, object: Option<Object>) -> EntityNodeData {
    let kind = if object.is_some() {
        EntityNodeKind::Entity
    } else {
        EntityNodeKind::Unresolved
    };
    EntityNodeData {
        target,
        object,
        kind,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityGraphExplorer {
    model: GraphModel<EntityNodeData, EntityEdgeData>,
    pub mode: GraphMode,
    pub limits: ExplorerLimits,
    pub root: EntityTarget,
    expanded: BTreeSet<NodeId>,
    loading: BTreeSet<NodeId>,
    // Expansion provenance, separate from semantic edge direction, drives collapse.
    expansion_nodes: BTreeMap<NodeId, BTreeSet<NodeId>>,
    expansion_edges: BTreeMap<NodeId, BTreeSet<EdgeId>>,
}

impl EntityGraphExplorer {
    pub fn new(root: EntityTarget, mode: GraphMode, limits: ExplorerLimits) -> Self {
        let mut this = Self {
            model: GraphModel::default(),
            mode,
            limits,
            root: root.clone(),
            expanded: BTreeSet::new(),
            loading: BTreeSet::new(),
            expansion_nodes: BTreeMap::new(),
            expansion_edges: BTreeMap::new(),
        };
        this.model
            .upsert_node(make_node(entity_data(root, None), None));
        this
    }
    pub fn model(&self) -> &GraphModel<EntityNodeData, EntityEdgeData> {
        &self.model
    }
    pub fn model_mut(&mut self) -> &mut GraphModel<EntityNodeData, EntityEdgeData> {
        &mut self.model
    }
    pub fn is_expanded(&self, id: &NodeId) -> bool {
        self.expanded.contains(id)
    }
    pub fn is_loading(&self, id: &NodeId) -> bool {
        self.loading.contains(id)
    }
    pub fn set_object(&mut self, target: &EntityTarget, object: Object) {
        if let Some(node) = self.model.node_mut(&node_id(target)) {
            node.data.object = Some(object);
            node.data.kind = EntityNodeKind::Entity;
        }
    }
    pub fn begin_expand(&mut self, id: &NodeId) -> Option<ExpansionRequest> {
        if self.expanded.contains(id)
            || self.loading.contains(id)
            || self.model.nodes().count() >= self.limits.max_nodes
        {
            return None;
        }
        let node = self.model.node(id)?;
        if matches!(node.data.kind, EntityNodeKind::Overflow { .. }) {
            return None;
        }
        let request = ExpansionRequest {
            node: id.clone(),
            target: node.data.target.clone(),
            mode: self.mode,
            limit: self.limits.fan_out.saturating_add(1),
        };
        self.loading.insert(id.clone());
        Some(request)
    }
    pub fn fail_expansion(&mut self, id: &NodeId) {
        self.loading.remove(id);
    }
    pub fn apply_expansion(&mut self, id: &NodeId, result: ExpansionResult) {
        self.loading.remove(id);
        if self.model.node(id).is_none() {
            return;
        }
        let current = self.model.nodes().count();
        let mut new_count = 0;
        let unique_new = result
            .nodes
            .iter()
            .map(|node| node_id(&node.target))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|id| self.model.node(id).is_none())
            .count();
        let available = self.limits.max_nodes.saturating_sub(current);
        let needs_overflow =
            unique_new > available || unique_new > self.limits.fan_out || result.remaining_hint > 0;
        let budget = available
            .saturating_sub(usize::from(needs_overflow))
            .min(self.limits.fan_out);
        let mut owned = BTreeSet::new();
        let mut omitted = result.remaining_hint;
        for data in result.nodes {
            let target = node_id(&data.target);
            if self.model.node(&target).is_none() {
                if new_count >= budget {
                    omitted += 1;
                    continue;
                }
                new_count += 1;
            }
            owned.insert(target.clone());
            if let Some(existing) = self.model.node_mut(&target) {
                if data.object.is_some() {
                    existing.data = data;
                }
            } else {
                self.model.upsert_node(make_node(data, Some(id.clone())));
            }
        }
        let mut owned_edges = BTreeSet::new();
        for edge in result.edges {
            let source = node_id(&edge.source);
            let target = node_id(&edge.target);
            if self.model.node(&source).is_none() || self.model.node(&target).is_none() {
                continue;
            }
            let (edge_id, style) = match &edge.kind {
                EntityEdgeKind::Parent => (
                    EdgeId(format!("parent:{target}->{source}")),
                    EdgeStyle {
                        path: EdgePathStyle::Bezier,
                        source_anchor: EdgeAnchor::Side(EdgeSide::Bottom),
                        target_anchor: EdgeAnchor::Side(EdgeSide::Top),
                        ..EdgeStyle::default()
                    },
                ),
                EntityEdgeKind::Relation { relation_id, label } => (
                    EdgeId(format!("rel:{relation_id}:{source}->{target}")),
                    EdgeStyle {
                        path: EdgePathStyle::Bezier,
                        label: Some(label.clone()),
                        class: Some(format!(
                            "semantic-graph-relation-{}",
                            stable_color(relation_id)
                        )),
                        ..EdgeStyle::default()
                    },
                ),
            };
            owned_edges.insert(edge_id.clone());
            if self.model.edges().all(|edge| edge.id != edge_id) {
                let _ = self.model.insert_edge(GraphEdge {
                    id: edge_id,
                    source,
                    target,
                    data: EntityEdgeData { kind: edge.kind },
                    style,
                });
            }
        }
        if omitted > 0 && self.model.nodes().count() < self.limits.max_nodes {
            let overflow_id = NodeId(format!("overflow:{id}"));
            let target = EntityTarget::default_collection(overflow_id.0.clone());
            let mut node = make_node(
                EntityNodeData {
                    target,
                    object: None,
                    kind: EntityNodeKind::Overflow {
                        parent: id.clone(),
                        remaining_hint: omitted,
                    },
                },
                Some(id.clone()),
            );
            node.id = overflow_id.clone();
            self.model.upsert_node(node);
            owned.insert(overflow_id.clone());
            let edge_id = EdgeId(format!("overflow:{id}"));
            owned_edges.insert(edge_id.clone());
            let _ = self.model.insert_edge(GraphEdge {
                id: edge_id,
                source: id.clone(),
                target: overflow_id,
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
        self.expanded.insert(id.clone());
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
        self.expanded.remove(id);
        self.loading.remove(id);
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
            self.expanded.remove(&id);
            self.loading.remove(&id);
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
            if let Some(item) = self.model.node_mut(&node) {
                item.layout_parent = Some(parent);
            }
        }
    }
    pub fn set_mode(&mut self, mode: GraphMode) {
        let object = self
            .model
            .node(&node_id(&self.root))
            .and_then(|node| node.data.object.clone());
        *self = Self::new(self.root.clone(), mode, self.limits);
        if let Some(object) = object {
            let root = self.root.clone();
            self.set_object(&root, object);
        }
    }
    pub fn ancestors_request(&self, id: &NodeId) -> Option<EntityTarget> {
        let node = self.model.node(id)?;
        let parent = node.data.object.as_ref()?.get(ATTR_PARENT)?.as_str()?;
        let target = EntityTarget::new(node.data.target.collection.clone(), parent);
        (self.model.node(&node_id(&target)).is_none()).then_some(target)
    }
    pub fn apply_ancestor(&mut self, child: &NodeId, data: EntityNodeData) {
        if self.model.nodes().count() >= self.limits.max_nodes {
            return;
        }
        let parent = node_id(&data.target);
        let Some(child_target) = self.model.node(child).map(|node| node.data.target.clone()) else {
            return;
        };
        self.model.upsert_node(make_node(data.clone(), None));
        if let Some(node) = self.model.node_mut(child) {
            node.layout_parent = Some(parent.clone());
        }
        self.expansion_nodes
            .entry(child.clone())
            .or_default()
            .insert(parent.clone());
        let edge_id = EdgeId(format!("parent:{child}->{parent}"));
        self.expansion_edges
            .entry(child.clone())
            .or_default()
            .insert(edge_id.clone());
        let _ = self.model.insert_edge(GraphEdge {
            id: edge_id,
            source: parent,
            target: node_id(&child_target),
            data: EntityEdgeData {
                kind: EntityEdgeKind::Parent,
            },
            style: EdgeStyle {
                source_anchor: EdgeAnchor::Side(EdgeSide::Bottom),
                target_anchor: EdgeAnchor::Side(EdgeSide::Top),
                ..EdgeStyle::default()
            },
        });
    }
}

fn make_node(data: EntityNodeData, parent: Option<NodeId>) -> GraphNode<EntityNodeData> {
    GraphNode {
        id: node_id(&data.target),
        order_key: data
            .object
            .as_ref()
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
                .map(|node| entity_data(target(node), Some(Object::new())))
                .collect(),
            edges: nodes
                .iter()
                .map(|node| ExpansionEdge {
                    source: target(source),
                    target: target(node),
                    kind: EntityEdgeKind::Parent,
                })
                .collect(),
            remaining_hint: 0,
        }
    }
    #[test]
    fn seed_expand_deduplicate_and_switch_mode() {
        let mut graph = EntityGraphExplorer::new(
            target("root"),
            GraphMode::Hierarchy,
            ExplorerLimits::default(),
        );
        assert_eq!(graph.model.nodes().count(), 1);
        assert!(graph.begin_expand(&id("root")).is_some());
        assert!(graph.begin_expand(&id("root")).is_none());
        graph.apply_expansion(&id("root"), fixture(&["a", "b", "a"], "root"));
        assert_eq!(graph.model.nodes().count(), 3);
        assert_eq!(graph.model.edges().count(), 2);
        graph.set_object(&target("root"), Object::new());
        graph.set_mode(GraphMode::Relations);
        assert_eq!(graph.model.nodes().count(), 1);
        assert!(graph.model.node(&id("root")).unwrap().data.object.is_some());
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
            let mut graph = EntityGraphExplorer::new(target("root"), GraphMode::Both, limits);
            graph.apply_expansion(&id("root"), fixture(&["a", "b", "c", "d"], "root"));
            assert!(graph.model.nodes().count() <= limits.max_nodes);
            assert!(
                graph
                    .model
                    .nodes()
                    .any(|node| matches!(node.data.kind, EntityNodeKind::Overflow { .. }))
            );
        }
    }
    #[test]
    fn collapse_keeps_shared_nodes_and_cyclic_relation_nodes() {
        let mut graph = EntityGraphExplorer::new(
            target("root"),
            GraphMode::Relations,
            ExplorerLimits::default(),
        );
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
            EntityGraphExplorer::new(target("root"), GraphMode::Both, ExplorerLimits::default());
        let mut root = Object::new();
        root.insert(ATTR_PARENT, Value::String("parent".into()));
        graph.set_object(&target("root"), root);
        let parent = graph.ancestors_request(&id("root")).unwrap();
        graph.apply_ancestor(&id("root"), entity_data(parent, None));
        assert_eq!(
            graph.model.node(&id("root")).unwrap().layout_parent,
            Some(id("parent"))
        );
        assert!(matches!(
            graph.model.node(&id("parent")).unwrap().data.kind,
            EntityNodeKind::Unresolved
        ));
    }

    #[test]
    fn collapse_reparents_shared_survivors_after_parent_removal() {
        let mut graph =
            EntityGraphExplorer::new(target("root"), GraphMode::Both, ExplorerLimits::default());
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
}
