use std::collections::HashMap;

pub type AnyError = anyhow::Error;

pub type Fields = HashMap<String, serde_json::Value>;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timestamp(u64);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeId(uuid::Uuid);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct RelationId(uuid::Uuid);

pub type PluginId = String;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Node<D = Fields> {
    pub kind: String,
    pub id: NodeId,
    pub uri: String,
    pub plugin: PluginId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub data: D,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Relation<D = Fields> {
    pub kind: String,
    pub id: RelationId,
    pub source_node: NodeId,
    pub target_node: NodeId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub data: D,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct NodeItem<D = Fields> {
    pub node: Node<D>,
    pub relations: Vec<RelationItem<D>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RelationItem<D = Fields> {
    pub relation: Relation<D>,
    pub source: Node<D>,
    pub target: Node<D>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Event {
    NodeCreated(Node),
    NodeUpdated { id: NodeId, data: Fields },
    NodeDeleted { id: NodeId },
    RelationCreated(Relation),
    RelationUpdated { id: RelationId, data: Fields },
    RelationDeleted { id: RelationId },
    Batch(Vec<Event>),
}
