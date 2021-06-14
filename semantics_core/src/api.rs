use std::collections::HashMap;

use crate::{AnyError, Node, NodeId, NodeItem, NodePage, NodeQuery, Relation, RelationId, RelationPage, RelationQuery, db::DbEvent};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SimpleHttpRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SimpleHttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Query {
    // Node
    Nodes(NodeQuery),
    NodeMerge(Node),
    NodeDelete(NodeId),
    // Relation
    Relations(RelationQuery),
    RelationMerge(Relation),
    RelationDelete(RelationId),
    // Db - other
    Batch(Vec<DbEvent>),

    Import{
        items: Vec<NodeItem>,
        import_media: bool,
    },

    /// Execute an HTTP request.
    HttpFetch(SimpleHttpRequest),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct QueryWithId {
    pub id: u64,
    pub query: Query,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Reply {
    Nodes(NodePage),
    NodeUpsert,
    NodeDelete,
    Relations(RelationPage),
    RelationUpsert,
    RelationDelete,
    Batch,
    Import,

    HttpFetch(SimpleHttpResponse),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ApiError {
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum ApiResponse {
    Ok(Reply),
    Err(ApiError),
}

pub trait ApiClientExecutor {
    type Future: std::future::Future<Output = Result<Reply, AnyError>>;
    fn execute(&self, query: Query) -> Self::Future;
}

#[derive(Clone)]
pub struct ApiClient<E: ApiClientExecutor> {
    exec: E,
}

impl<E: ApiClientExecutor> ApiClient<E> {
    pub fn new(exec: E) -> Self {
        Self { exec }
    }

    pub async fn nodes(&self, query: NodeQuery) -> Result<NodePage, AnyError> {
        match self.exec.execute(Query::Nodes(query)).await {
            Ok(Reply::Nodes(page)) => Ok(page),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn node_merge(&self, node: Node) -> Result<(), AnyError> {
        match self.exec.execute(Query::NodeMerge(node)).await {
            Ok(Reply::NodeUpsert) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn node_delete(&self, id: NodeId) -> Result<(), AnyError> {
        match self.exec.execute(Query::NodeDelete(id)).await {
            Ok(Reply::NodeDelete) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn relations(&self, query: RelationQuery) -> Result<RelationPage, AnyError> {
        match self.exec.execute(Query::Relations(query)).await {
            Ok(Reply::Relations(page)) => Ok(page),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn relation_upsert(&self, relation: Relation) -> Result<(), AnyError> {
        match self.exec.execute(Query::RelationMerge(relation)).await {
            Ok(Reply::RelationUpsert) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn relation_delete(&self, id: RelationId) -> Result<(), AnyError> {
        match self.exec.execute(Query::RelationDelete(id)).await {
            Ok(Reply::RelationDelete) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn batch(&self, events: Vec<DbEvent>) -> Result<(), AnyError> {
        match self.exec.execute(Query::Batch(events)).await {
            Ok(Reply::Batch) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }
}
