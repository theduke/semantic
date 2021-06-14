pub mod memory;

use std::sync::Arc;

use crate::{
    AnyError, Node, NodeId, NodeItem, NodeQuery, Page, Relation, RelationId, RelationItem,
    RelationQuery,
    Patch,
};

pub type DbFuture<T> = futures::future::BoxFuture<'static, Result<T, AnyError>>;

#[derive(Debug)]
pub struct NotFoundError {
    message: String,
}

impl NotFoundError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for NotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Not found: {}", self.message)
    }
}

impl std::error::Error for NotFoundError {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum DbEvent {
    NodeMerged(Node),
    NodeUpdated { id: NodeId, data: Patch },
    NodeDeleted { id: NodeId },
    RelationMerged(Relation),
    RelationUpdated { id: RelationId, data: Patch },
    RelationDeleted { id: RelationId },
    Batch(Vec<DbEvent>),
}

impl DbEvent {
    pub fn flatten_into(self, events: &mut Vec<DbEvent>) {
        match self {
            DbEvent::Batch(subevents) => {
                subevents.into_iter().for_each(|ev| ev.flatten_into(events))
            }
            other => events.push(other),
        }
    }

    pub fn flatten(self) -> Vec<DbEvent> {
        let mut evs = Vec::new();
        self.flatten_into(&mut evs);
        evs
    }

    pub fn is_node(&self) -> bool {
        match self {
            DbEvent::NodeMerged(_) => true,
            DbEvent::NodeUpdated { .. } => true,
            DbEvent::NodeDeleted { .. } => true,
            _ => false,
        }
    }

    pub fn nodes_merge_batch(items: Vec<NodeItem>) -> Self {
        let mut nodes = Vec::new();
        let mut relations = Vec::new();
        for item in items {
            item.flatten_into(&mut nodes, &mut relations);
        }
        let events = nodes
            .into_iter()
            .map(Self::NodeMerged)
            .chain(relations.into_iter().map(Self::RelationMerged))
            .collect();
        Self::Batch(events)
    }

    pub fn as_events(self) -> Vec<Self> {
        match self {
            DbEvent::NodeMerged(_) => todo!(),
            DbEvent::NodeUpdated { id, data } => todo!(),
            DbEvent::NodeDeleted { id } => todo!(),
            DbEvent::RelationMerged(_) => todo!(),
            DbEvent::RelationUpdated { id, data } => todo!(),
            DbEvent::RelationDeleted { id } => todo!(),
            DbEvent::Batch(batch) => batch,
        }
    }
}

pub trait Db {
    fn node(&self, id: NodeId) -> DbFuture<Node>;
    fn node_by_uri(&self, uri: String) -> DbFuture<Node>;
    fn nodes(&self, query: NodeQuery) -> DbFuture<Page<NodeItem>>;

    /// Merge node data.
    /// If a node with the same URI exists, the data will be merged.
    /// Invidivudal fields will be overwritten, old fields will be retained.
    ///
    /// See [Self::node_patch] for more fine-grained patching.
    fn node_merge(&self, node: Node) -> DbFuture<()>;

    /// Update a node with the given Patch.
    fn node_patch(&self, id: NodeId, data: Patch) -> DbFuture<Node>;

    /// Delete a node.
    fn node_delete(&self, id: NodeId) -> DbFuture<()>;

    fn relation(&self, id: RelationId) -> DbFuture<Relation>;
    fn relations(&self, query: RelationQuery) -> DbFuture<Page<RelationItem>>;
    fn relation_merge(&self, relation: Relation) -> DbFuture<()>;
    fn relation_patch(&self, id: RelationId, data: Patch) -> DbFuture<Relation>;
    fn relation_delete(&self, id: RelationId) -> DbFuture<()>;

    fn batch(&self, batch: Vec<DbEvent>) -> DbFuture<()>;
}

pub type DynDb = Arc<dyn Db + Send + Sync>;

mod tests {
    use crate::{map, Fields, NodeKind};

    use super::*;

    fn make_node(
        kind: impl Into<NodeKind>,
        uri: impl Into<String>,
        data: impl Into<Fields>,
    ) -> Node {
        let now = crate::Timestamp::now();
        Node {
            kind: kind.into(),
            id: NodeId(uuid::Uuid::new_v4()),
            uri: uri.into(),
            plugin: "test".into(),
            created_at: now.clone(),
            updated_at: now,
            data: data.into(),
        }
    }

    pub fn test_db(db: impl Db) {
        futures::executor::block_on(test_db_async(db));
    }

    pub async fn test_db_async(db: impl Db) {
        // Merge with same uri.
        let node = make_node("kind", "test/a", map! { "a": 1, "b": true });
        db.node_merge(node.clone()).await.unwrap();
        let node2 = make_node("kind", "test/a", map! { "a": 3, "c": 22 });
        db.node_merge(node2.clone()).await.unwrap();
        let node3 = db.node(node.id).await.unwrap();
        assert_eq!(
            node3.data,
            map! {
                "a": 3, "b": true, "c": 22,
            }
        );
        assert!(db.node(node2.id).await.is_err());
    }
}

pub use tests::{test_db, test_db_async};
