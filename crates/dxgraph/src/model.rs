//! Serializable content-agnostic graph model. Array order preserves insertion order.
use crate::{
    edge::EdgeStyle,
    geometry::{Point, Rect, Size},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;
macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}
id_type!(NodeId);
id_type!(EdgeId);
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphNode<N> {
    pub id: NodeId,
    pub data: N,
    pub position: Option<Point>,
    pub size_hint: Size,
    pub pinned: bool,
    pub layout_parent: Option<NodeId>,
    pub order_key: Option<String>,
}
impl<N> GraphNode<N> {
    pub fn new(id: impl Into<NodeId>, data: N) -> Self {
        Self {
            id: id.into(),
            data,
            position: None,
            size_hint: Size::new(160.0, 64.0),
            pinned: false,
            layout_parent: None,
            order_key: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge<E> {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub data: E,
    pub style: EdgeStyle,
}
impl<E> GraphEdge<E> {
    pub fn new(
        id: impl Into<EdgeId>,
        source: impl Into<NodeId>,
        target: impl Into<NodeId>,
        data: E,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            data,
            style: EdgeStyle::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphModel<N, E> {
    nodes: IndexMap<NodeId, GraphNode<N>>,
    edges: IndexMap<EdgeId, GraphEdge<E>>,
}
impl<N, E> Default for GraphModel<N, E> {
    fn default() -> Self {
        Self {
            nodes: IndexMap::new(),
            edges: IndexMap::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    #[error("Unknown graph node: {0}")]
    UnknownNode(NodeId),
    #[error("Graph node already exists: {0}")]
    DuplicateNode(NodeId),
    #[error("Graph edge already exists: {0}")]
    DuplicateEdge(EdgeId),
}
impl<N, E> GraphModel<N, E> {
    pub fn insert_node(&mut self, node: GraphNode<N>) -> Result<(), GraphError> {
        if self.nodes.contains_key(&node.id) {
            return Err(GraphError::DuplicateNode(node.id));
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }
    /// Replace content and metadata, preserving the existing position and pin.
    pub fn upsert_node(&mut self, mut node: GraphNode<N>) -> Option<GraphNode<N>> {
        if let Some(existing) = self.nodes.get(&node.id) {
            node.position = existing.position;
            node.pinned = existing.pinned;
        }
        self.nodes.insert(node.id.clone(), node)
    }
    pub fn remove_node(&mut self, id: &NodeId) -> Option<GraphNode<N>> {
        let removed = self.nodes.shift_remove(id)?;
        self.edges
            .retain(|_, edge| &edge.source != id && &edge.target != id);
        for node in self.nodes.values_mut() {
            if node.layout_parent.as_ref() == Some(id) {
                node.layout_parent = None;
            }
        }
        Some(removed)
    }
    pub fn insert_edge(&mut self, edge: GraphEdge<E>) -> Result<(), GraphError> {
        for id in [&edge.source, &edge.target] {
            if !self.nodes.contains_key(id) {
                return Err(GraphError::UnknownNode(id.clone()));
            }
        }
        if self.edges.contains_key(&edge.id) {
            return Err(GraphError::DuplicateEdge(edge.id));
        }
        self.edges.insert(edge.id.clone(), edge);
        Ok(())
    }
    pub fn remove_edge(&mut self, id: &EdgeId) -> Option<GraphEdge<E>> {
        self.edges.shift_remove(id)
    }
    pub fn node(&self, id: &NodeId) -> Option<&GraphNode<N>> {
        self.nodes.get(id)
    }
    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut GraphNode<N>> {
        self.nodes.get_mut(id)
    }
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = &GraphNode<N>> {
        self.nodes.values()
    }
    pub fn edges(&self) -> impl ExactSizeIterator<Item = &GraphEdge<E>> {
        self.edges.values()
    }
    pub fn incident_edges<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a GraphEdge<E>> {
        self.edges
            .values()
            .filter(move |e| &e.source == id || &e.target == id)
    }
    pub fn neighbors<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a NodeId> {
        self.nodes.keys().filter(move |other| {
            self.edges.values().any(|e| {
                (&e.source == id && &e.target == *other) || (&e.target == id && &e.source == *other)
            })
        })
    }
    pub fn set_position(&mut self, id: &NodeId, position: Point) -> Result<(), GraphError> {
        self.node_mut(id)
            .ok_or_else(|| GraphError::UnknownNode(id.clone()))?
            .position = Some(position);
        Ok(())
    }
    pub fn set_pinned(&mut self, id: &NodeId, pinned: bool) -> Result<(), GraphError> {
        self.node_mut(id)
            .ok_or_else(|| GraphError::UnknownNode(id.clone()))?
            .pinned = pinned;
        Ok(())
    }
    pub fn retain_nodes(&mut self, mut predicate: impl FnMut(&GraphNode<N>) -> bool) {
        let removed: Vec<_> = self
            .nodes
            .values()
            .filter(|node| !predicate(node))
            .map(|node| node.id.clone())
            .collect();
        for id in removed {
            self.remove_node(&id);
        }
    }
    pub fn bounds(&self, sizes: &IndexMap<NodeId, Size>) -> Option<Rect> {
        self.nodes
            .values()
            .filter_map(|node| {
                node.position.map(|position| {
                    Rect::new(
                        position,
                        sizes.get(&node.id).copied().unwrap_or(node.size_hint),
                    )
                })
            })
            .reduce(Rect::union)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_invariants() {
        let mut graph = GraphModel::<(), ()>::default();
        assert_eq!(
            graph.insert_edge(GraphEdge::new("e", "a", "b", ())),
            Err(GraphError::UnknownNode("a".into()))
        );
        for id in ["a", "b", "c"] {
            graph.insert_node(GraphNode::new(id, ())).unwrap();
        }
        assert!(graph.insert_node(GraphNode::new("a", ())).is_err());
        graph
            .insert_edge(GraphEdge::new("e", "a", "b", ()))
            .unwrap();
        assert!(
            graph
                .insert_edge(GraphEdge::new("e", "a", "b", ()))
                .is_err()
        );
        assert_eq!(graph.incident_edges(&"a".into()).count(), 1);
        assert_eq!(
            graph.neighbors(&"a".into()).collect::<Vec<_>>(),
            vec![&NodeId::from("b")]
        );
        graph
            .set_position(&"a".into(), Point::new(10.0, 20.0))
            .unwrap();
        graph.set_pinned(&"a".into(), true).unwrap();
        assert!(graph.set_pinned(&"missing".into(), true).is_err());
        graph.upsert_node(GraphNode::new("a", ()));
        assert!(graph.node(&"a".into()).unwrap().pinned);
        assert_eq!(
            graph.node(&"a".into()).unwrap().position,
            Some(Point::new(10.0, 20.0))
        );
        assert_eq!(
            graph.bounds(&IndexMap::new()).unwrap().size,
            Size::new(160.0, 64.0)
        );
        graph.node_mut(&"b".into()).unwrap().layout_parent = Some("a".into());
        graph.remove_node(&"a".into());
        assert_eq!(graph.edges().count(), 0);
        assert_eq!(graph.node(&"b".into()).unwrap().layout_parent, None);
        graph
            .insert_edge(GraphEdge::new("bc", "b", "c", ()))
            .unwrap();
        assert!(graph.remove_edge(&"bc".into()).is_some());
        graph.retain_nodes(|n| n.id == NodeId::from("b"));
        assert_eq!(graph.nodes().count(), 1);
        let encoded = serde_json::to_string(&graph).unwrap();
        assert_eq!(
            serde_json::from_str::<GraphModel<(), ()>>(&encoded).unwrap(),
            graph
        );
    }
}
