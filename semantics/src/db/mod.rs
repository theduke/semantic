pub mod memory;

use semantics_core::{AnyError, Node, NodeId, Relation, RelationId};

pub type DbFuture<T> = futures::future::BoxFuture<'static, Result<T, AnyError>>;

pub trait Db {
    fn node(&self, id: NodeId) -> DbFuture<Node>;
    fn node_create(&self, node: Node) -> DbFuture<()>;
    fn node_delete(&self, id: NodeId) ->  DbFuture<()>;

    fn relation(&self, id: RelationId) -> DbFuture<Relation>;
    fn relation_create(&self, relation: Relation) -> DbFuture<()>;
    fn relation_delete(&self, id: RelationId) -> DbFuture<()>;
}
