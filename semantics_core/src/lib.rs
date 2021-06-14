pub mod api;
pub mod db;

pub mod schema;

use std::collections::HashMap;

pub use serde_json::Value;

pub type AnyError = anyhow::Error;

pub type Fields = HashMap<String, Value>;

// pub use json_patch::Patch;
//

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum PatchOp {
    Set {
        field: String,
        value: Value,
    },
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Patch {
    ops: Vec<PatchOp>,
}

impl Patch {
    pub fn new() -> Self { Self { ops: Vec::new() } }

    pub fn with_set(mut self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.ops.push(PatchOp::Set{
            field: field.into(),
            value: value.into(),
        });
        self
    }

    pub fn apply(self, data: &mut Fields) {
        for op in self.ops {
            match op {
                PatchOp::Set { field, value } => {
                    data.insert(field, value);
                }
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timestamp(u64);

impl Timestamp {
    pub fn now() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        Self(t as u64)
    }
}

impl From<chrono::DateTime<chrono::Utc>> for Timestamp {
    fn from(v: chrono::DateTime<chrono::Utc>) -> Self {
        Self(v.timestamp() as u64)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeId(uuid::Uuid);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct RelationId(uuid::Uuid);

pub type NodeKind = String;

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

pub type RelationKind = String;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Relation<D = Fields> {
    pub kind: RelationKind,
    pub id: RelationId,
    pub uri: Option<String>,
    pub plugin: PluginId,
    pub source_node: NodeId,
    pub target_node: NodeId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub data: D,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct NodeItem<D = Fields> {
    pub node: Node<D>,
    pub joins: HashMap<String, Vec<RelationItem<D>>>,
}

impl NodeItem {
    pub fn flatten(self) -> (Vec<Node>, Vec<Relation>) {
        let mut nodes = Vec::new();
        let mut relations = Vec::new();
        self.flatten_into(&mut nodes, &mut relations);
        (nodes, relations)
    }

    pub fn flatten_into(self, nodes: &mut Vec<Node>, relations: &mut Vec<Relation>) {
        nodes.push(self.node);
        self.joins
            .into_iter()
            .map(|(_name, relations)| relations)
            .flatten()
            .for_each(|rel| rel.flatten_into(nodes, relations));
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RelationItem<D = Fields> {
    pub relation: Relation<D>,
    pub source: Option<NodeItem<D>>,
    pub target: Option<NodeItem<D>>,
}

impl RelationItem {
    fn flatten_into(self, nodes: &mut Vec<Node>, relations: &mut Vec<Relation>) {
        relations.push(self.relation);
        if let Some(source) = self.source {
            source.flatten_into(nodes, relations);
        }
        if let Some(target) = self.target {
            target.flatten_into(nodes, relations);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Page<D> {
    pub items: Vec<D>,
    pub next_cursor: Option<uuid::Uuid>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum NodeFilter {
    All,
    NodeKind(NodeKind),
    Id(NodeId),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum RelationDirection {
    Incoming,
    Outgoing,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum NodeJoinFilter {
    Direction(RelationDirection),
    NodeKind(NodeKind),
    RelationKind(RelationKind),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct NodeJoin {
    pub name: String,
    pub filter: NodeJoinFilter,
    pub limit: u32,
    /// If true, load the target node.
    pub with_node: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct NodeQuery {
    pub filter: NodeFilter,
    pub joins: Vec<NodeJoin>,
    pub cursor: Option<NodeId>,
    pub limit: u32,
}

pub type NodePage = Page<NodeItem>;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum RelationFilter {
    All,
    Kind(RelationKind),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RelationQuery {
    pub filter: RelationFilter,
    pub cursor: Option<NodeId>,
    pub join_source: bool,
    pub join_target: bool,
    pub limit: u32,
}

pub type RelationPage = Page<RelationItem>;

#[macro_export]
macro_rules! map {
    (
        $( $key:literal : $value:expr ),* $( , )?
    ) => {
        {

            let mut fields = $crate::Fields::new();
            $(
                fields.insert($key.into(), $value.into());
            )*
            fields
        }

    };
}
