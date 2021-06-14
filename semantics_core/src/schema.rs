use crate::{NodeKind, PluginId, RelationKind};

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum ValueType {
    Bool,
    Int,
    Float,
    String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct FieldSchema {
  pub name: String,
  pub label: String,
  pub data_type: ValueType,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct DataFieldSchema {
  pub field_name: String,
  pub list: bool,
  pub required: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct NodeSchema {
  pub kind: NodeKind,
  pub plugin: PluginId,
  pub extends: Option<NodeKind>,
  pub fields: Vec<DataFieldSchema>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RelationSchema {
  pub kind: RelationKind,
  pub plugin: PluginId,
  pub parent: Option<RelationKind>,
  pub fields: Vec<DataFieldSchema>,
  pub from_nodes: Option<Vec<NodeKind>>,
  pub to_nodes: Option<Vec<NodeKind>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct PluginSchema {
  pub name: String,
  pub label: String,
  pub fields: Vec<FieldSchema>,
  pub nodes: Vec<NodeSchema>,
  pub relations: Vec<RelationSchema>,
}

