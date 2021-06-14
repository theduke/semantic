use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::ready,
    sync::{Arc, RwLock},
};

use futures::FutureExt;

use crate::{
    AnyError, Fields, Node, NodeFilter, NodeId, NodeItem, NodeQuery, Page, Patch, Relation,
    RelationFilter, RelationId, RelationItem, RelationQuery, Timestamp,
};

use super::{DbEvent, DbFuture, NotFoundError};

type SharedStr = Arc<str>;

type SharedFields = HashMap<SharedStr, serde_json::Value>;

struct MemoryNode {
    kind: SharedStr,
    id: NodeId,
    // TODO: use SharedStr
    uri: String,
    plugin: SharedStr,
    created_at: Timestamp,
    updated_at: Timestamp,
    data: SharedFields,

    relations_incoming: HashSet<RelationId>,
    relations_outgoing: HashSet<RelationId>,
}

struct MemoryRelation {
    kind: SharedStr,
    id: RelationId,
    // TODO: use SharedStr
    uri: Option<String>,
    plugin: SharedStr,
    source_node: NodeId,
    target_node: NodeId,
    created_at: Timestamp,
    updated_at: Timestamp,
    data: SharedFields,
}

#[derive(Clone)]
enum DbElementId {
    Node(NodeId),
    Relation(RelationId),
}

struct State {
    nodes: HashMap<NodeId, MemoryNode>,
    relations: HashMap<RelationId, MemoryRelation>,

    /// Lookup table that maps canonical URIs to ids.
    uris: BTreeMap<String, DbElementId>,

    /// String interner.
    strings: HashMap<String, SharedStr>,
}

impl State {
    fn make_string(&mut self, value: String) -> SharedStr {
        self.strings.get(&value).cloned().unwrap_or_else(|| {
            let shared: SharedStr = Arc::from(value.as_str());
            self.strings.insert(value, shared.clone());
            shared
        })
    }

    fn fields_to_memory(&mut self, data: Fields) -> SharedFields {
        data.into_iter()
            .map(|(key, value)| (self.make_string(key), value))
            .collect()
    }

    fn fields_from_memory(data: &SharedFields) -> Fields {
        data.into_iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    fn node_to_memory(&mut self, node: Node) -> MemoryNode {
        MemoryNode {
            kind: self.make_string(node.kind),
            id: node.id,
            uri: node.uri,
            plugin: self.make_string(node.plugin),
            created_at: node.created_at,
            updated_at: node.updated_at,
            data: self.fields_to_memory(node.data),
            relations_incoming: HashSet::new(),
            relations_outgoing: HashSet::new(),
        }
    }

    fn node_from_memory(node: &MemoryNode) -> Node {
        Node {
            kind: node.kind.to_string(),
            id: node.id,
            uri: node.uri.clone(),
            plugin: node.plugin.to_string(),
            created_at: node.created_at,
            updated_at: node.updated_at,
            data: Self::fields_from_memory(&node.data),
        }
    }

    fn node(&self, id: NodeId) -> Result<&MemoryNode, NotFoundError> {
        self.nodes
            .get(&id)
            .ok_or_else(|| NotFoundError::new("Node not found"))
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut MemoryNode, NotFoundError> {
        self.nodes
            .get_mut(&id)
            .ok_or_else(|| NotFoundError::new("Node not found"))
    }

    // fn node_insert(&mut self, node: Node) {
    //     // TODO: uniqueness validation / return?
    //     self.uris.insert(node.uri.clone(), DbElementId::Node(node.id));
    //     let mem_node = self.node_to_memory(node);
    //     self.nodes.insert(mem_node.id, mem_node);
    // }

    fn node_merge(&mut self, node: Node) -> Result<Node, AnyError> {
        let mem_node = self.node_to_memory(node.clone());

        match self.uris.get(&mem_node.uri).cloned() {
            Some(DbElementId::Node(id)) => {
                let old_node = self.node_mut(id)?;
                old_node.data.extend(mem_node.data);

                return Ok(State::node_from_memory(&old_node));
            }
            Some(DbElementId::Relation(_)) => {
                return Err(anyhow::anyhow!(
                    "Can't create node: there exists a relation with the same URI - {}",
                    mem_node.uri
                ));
            }
            None => {}
        }

        self.uris
            .insert(mem_node.uri.clone(), DbElementId::Node(node.id));
        self.nodes.insert(mem_node.id, mem_node);
        Ok(node)
    }

    pub fn node_patch(&mut self, id: NodeId, data: Patch) -> Result<Node, AnyError> {
        let mem_node = self.node(id)?;
        let mut node = Self::node_from_memory(mem_node);
        data.apply(&mut node.data);

        let new_fields = self.fields_to_memory(node.data.clone());
        let mut mem_node = self.node_mut(id)?;
        mem_node.data = new_fields;

        Ok(node)
    }

    fn node_remove(&mut self, id: NodeId) -> Result<(Node, Vec<Relation>), AnyError> {
        let mem_node = self
            .nodes
            .remove(&id)
            .ok_or_else(|| NotFoundError::new("Node not found"))?;
        self.uris.remove(&mem_node.uri);

        // TODO: use BTreeMap::drain_filter once stabilized.

        let mut relations = Vec::new();

        self.relations.retain(|_id, rel| {
            if rel.source_node == id || rel.target_node == id {
                relations.push(Self::relation_from_memory(rel));
                false
            } else {
                true
            }
        });

        for rel in &relations {
            if let Some(uri) = &rel.uri {
                self.uris.remove(uri);
            }
        }

        let node = Self::node_from_memory(&mem_node);
        Ok((node, relations))
    }

    fn node_filter_apply(f: &NodeFilter, node: &MemoryNode) -> bool {
        match f {
            NodeFilter::All => true,
            NodeFilter::NodeKind(kind) => &*node.kind == kind,
            NodeFilter::Id(id) => &node.id == id,
        }
    }

    fn query_nodes(&self, query: NodeQuery) -> Result<Page<NodeItem>, AnyError> {
        let nodes = self
            .nodes
            .values()
            .filter(|node| Self::node_filter_apply(&query.filter, node));

        // FIXME: joins.
        let items = nodes
            .map(|node| NodeItem {
                node: Self::node_from_memory(node),
                joins: HashMap::new(),
            })
            .collect();

        Ok(Page {
            items,
            next_cursor: None,
        })
    }

    fn relation_to_memory(&mut self, rel: Relation) -> MemoryRelation {
        MemoryRelation {
            kind: self.make_string(rel.kind),
            id: rel.id,
            uri: rel.uri,
            created_at: rel.created_at,
            updated_at: rel.updated_at,
            data: self.fields_to_memory(rel.data),
            source_node: rel.source_node,
            target_node: rel.target_node,
            plugin: self.make_string(rel.plugin),
        }
    }

    fn relation_from_memory(rel: &MemoryRelation) -> Relation {
        Relation {
            kind: rel.kind.to_string(),
            id: rel.id,
            uri: rel.uri.clone(),
            plugin: rel.plugin.to_string(),
            source_node: rel.source_node,
            target_node: rel.target_node,
            created_at: rel.created_at,
            updated_at: rel.updated_at,
            data: Self::fields_from_memory(&rel.data),
        }
    }

    fn relation(&self, id: RelationId) -> Result<&MemoryRelation, NotFoundError> {
        self.relations
            .get(&id)
            .ok_or_else(|| NotFoundError::new("Relation not found"))
    }

    fn relation_mut(&mut self, id: RelationId) -> Result<&mut MemoryRelation, NotFoundError> {
        self.relations
            .get_mut(&id)
            .ok_or_else(|| NotFoundError::new("Relation not found"))
    }

    fn relation_insert(&mut self, rel: Relation) -> Result<(), AnyError> {
        // FIXME: proper insert/upsert logic.
        let _old = self.relation_remove(rel.id);
        let rel = self.relation_to_memory(rel);
        self.relations.insert(rel.id, rel);
        Ok(())
    }

    fn relation_merge(&mut self, rel: Relation) -> Result<Relation, AnyError> {
        let mem_relation = self.relation_to_memory(rel.clone());

        self.nodes
            .get_mut(&rel.source_node)
            .ok_or_else(|| NotFoundError::new("Source relation not found"))?
            .relations_outgoing
            .insert(rel.id);
        self.nodes
            .get_mut(&rel.target_node)
            .ok_or_else(|| NotFoundError::new("Target relation not found"))?
            .relations_incoming
            .insert(rel.id);

        if let Some(uri) = mem_relation.uri.clone() {
            match self.uris.get(&uri).cloned() {
                Some(DbElementId::Relation(id)) => {
                    let old_relation = self.relation_mut(id)?;
                    old_relation.data.extend(mem_relation.data);
                    // TODO: do we need to handle a change of target/source
                    // nodes separately?
                    old_relation.source_node = mem_relation.source_node;
                    old_relation.target_node = mem_relation.target_node;

                    return Ok(State::relation_from_memory(&old_relation));
                }
                Some(DbElementId::Node(_)) => {
                    return Err(anyhow::anyhow!(
                        "Can't create relation: there exists a relation with the same URI - {}",
                        uri
                    ));
                }
                None => {}
            }

            self.uris.insert(uri, DbElementId::Relation(rel.id));
        }

        self.relations.insert(mem_relation.id, mem_relation);
        Ok(rel)
    }

    fn relation_remove(&mut self, id: RelationId) -> Result<(), AnyError> {
        let rel = self
            .relations
            .remove(&id)
            .ok_or_else(|| NotFoundError::new("Relation not found"))?;

        self.nodes
            .get_mut(&rel.source_node)
            .map(|n| n.relations_outgoing.remove(&id));
        self.nodes
            .get_mut(&rel.target_node)
            .map(|n| n.relations_incoming.remove(&id));

        Ok(())
    }

    fn relation_filter_apply(f: &RelationFilter, rel: &MemoryRelation) -> bool {
        match f {
            RelationFilter::All => true,
            RelationFilter::Kind(k) => &*rel.kind == k,
        }
    }

    fn query_relations(&self, query: RelationQuery) -> Result<Page<RelationItem>, AnyError> {
        let rels = self
            .relations
            .values()
            .filter(|rel| Self::relation_filter_apply(&query.filter, rel));

        // FIXME: joins.
        let items = rels
            .map(|rel| {
                let source = if query.join_source {
                    self.nodes.get(&rel.source_node).map(|n| NodeItem {
                        node: Self::node_from_memory(n),
                        joins: Default::default(),
                    })
                } else {
                    None
                };

                let target = if query.join_target {
                    self.nodes.get(&rel.target_node).map(|n| NodeItem {
                        node: Self::node_from_memory(n),
                        joins: Default::default(),
                    })
                } else {
                    None
                };

                RelationItem {
                    relation: Self::relation_from_memory(rel),
                    source,
                    target,
                }
            })
            .collect();

        Ok(Page {
            items,
            next_cursor: None,
        })
    }

    fn apply_event(&mut self, ev: DbEvent) -> Result<(), AnyError> {
        match ev {
            DbEvent::NodeMerged(n) => {
                // TODO: uniqueness validation
                self.node_merge(n)?;
            }
            DbEvent::NodeUpdated { id, data } => {
                self.node_patch(id, data)?;
            }
            DbEvent::NodeDeleted { id } => {
                self.node_remove(id).ok();
            }
            DbEvent::RelationMerged(rel) => {
                // FIXME: handle errors.
                // disabled due to auto-deletion of relations on node deletion
                // not implemented yet.
                self.relation_insert(rel).ok();
            }
            DbEvent::RelationUpdated { id, data } => todo!(),
            DbEvent::RelationDeleted { id } => {
                self.relation_remove(id)?;
            }
            DbEvent::Batch(events) => {
                let mut flat = Vec::new();
                for ev in events {
                    ev.flatten_into(&mut flat);
                }

                // First , handle all nodes, then relations.
                for event in &flat {
                    if event.is_node() {
                        self.apply_event(event.clone())?;
                    }
                }

                for event in flat {
                    if !event.is_node() {
                        self.apply_event(event)?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct MemoryDb {
    state: Arc<RwLock<State>>,
}

impl MemoryDb {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State {
                nodes: HashMap::new(),
                relations: HashMap::new(),
                uris: BTreeMap::new(),
                strings: HashMap::new(),
            })),
        }
    }

    pub fn node(&self, id: NodeId) -> Result<Node, AnyError> {
        let state = self.state.read().unwrap();
        state
            .nodes
            .get(&id)
            .map(|n| State::node_from_memory(n))
            .ok_or_else(|| AnyError::from(NotFoundError::new("Node not found")))
    }

    pub fn node_by_uri(&self, uri: String) -> Result<Node, AnyError> {
        let state = self.state.read().unwrap();
        match state.uris.get(&uri) {
            Some(DbElementId::Node(id)) => Ok(State::node_from_memory(state.node(*id)?)),
            _ => Err(NotFoundError::new(format!("Node uri not found: {}", uri)).into()),
        }
    }

    pub fn nodes(&self, query: NodeQuery) -> Result<Page<NodeItem>, AnyError> {
        self.state.read().unwrap().query_nodes(query)
    }

    pub fn node_merge(&self, node: Node) -> Result<Node, AnyError> {
        self.state.write().unwrap().node_merge(node)
    }

    pub fn node_patch(&self, id: NodeId, data: Patch) -> Result<Node, AnyError> {
        self.state.write().unwrap().node_patch(id, data)
    }

    pub fn node_delete(&self, id: NodeId) -> Result<(), AnyError> {
        self.state.write().unwrap().node_remove(id)?;
        Ok(())
    }

    pub fn node_delete_retrieve(&self, id: NodeId) -> Result<(Node, Vec<Relation>), AnyError> {
        self.state.write().unwrap().node_remove(id)
    }

    pub fn relation(&self, id: RelationId) -> Result<Relation, AnyError> {
        let state = self.state.read().unwrap();
        state
            .relations
            .get(&id)
            .map(|n| State::relation_from_memory(n))
            .ok_or_else(|| AnyError::from(NotFoundError::new("Relation not found")))
    }

    fn relations(&self, query: RelationQuery) -> Result<Page<RelationItem>, AnyError> {
        self.state.read().unwrap().query_relations(query)
    }

    pub fn relation_merge(&self, relation: Relation) -> Result<Relation, AnyError> {
        self.state.write().unwrap().relation_merge(relation)
    }

    pub fn relation_patch(&self, relation: RelationId, data: Patch) -> Result<Relation, AnyError> {
        todo!()
    }

    pub fn relation_delete(&self, id: RelationId) -> Result<(), AnyError> {
        self.state.write().unwrap().relation_remove(id)
    }

    pub fn apply_event(&self, ev: super::DbEvent) -> Result<(), AnyError> {
        self.state.write().unwrap().apply_event(ev)
    }
}

impl super::Db for MemoryDb {
    fn node(&self, id: NodeId) -> DbFuture<Node> {
        ready(self.node(id)).boxed()
    }

    fn node_by_uri(&self, uri: String) -> DbFuture<Node> {
        ready(self.node_by_uri(uri)).boxed()
    }

    fn nodes(&self, query: NodeQuery) -> DbFuture<Page<NodeItem>> {
        ready(self.nodes(query)).boxed()
    }

    fn node_merge(&self, node: Node) -> DbFuture<()> {
        ready(self.node_merge(node).map(|_x| ())).boxed()
    }

    fn node_patch(&self, node: NodeId, data: Patch) -> DbFuture<Node> {
        ready(self.node_patch(node, data)).boxed()
    }

    fn node_delete(&self, id: NodeId) -> DbFuture<()> {
        ready(self.node_delete(id)).boxed()
    }

    fn relation(&self, id: RelationId) -> DbFuture<Relation> {
        ready(self.relation(id)).boxed()
    }

    fn relations(&self, query: RelationQuery) -> DbFuture<Page<RelationItem>> {
        ready(self.relations(query)).boxed()
    }

    fn relation_merge(&self, relation: Relation) -> DbFuture<()> {
        ready(self.relation_merge(relation).map(|_x| ())).boxed()
    }

    fn relation_patch(&self, relation: RelationId, data: Patch) -> DbFuture<Relation> {
        ready(self.relation_patch(relation, data)).boxed()
    }

    fn relation_delete(&self, id: RelationId) -> DbFuture<()> {
        ready(self.relation_delete(id)).boxed()
    }

    fn batch(&self, batch: Vec<DbEvent>) -> DbFuture<()> {
        let res = self.apply_event(DbEvent::Batch(batch));
        ready(res).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_db() {
        crate::db::test_db(MemoryDb::new());
    }
}
