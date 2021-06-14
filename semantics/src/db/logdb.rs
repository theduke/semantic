use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use futures::FutureExt;
use logfs::LogFs;
use semantics_core::{
    db::{self, memory::MemoryDb, Db, DbEvent, DbFuture},
    AnyError, Node, Patch, Relation,
};
use tokio::task::spawn_blocking;

struct State {
    event_index: u64,
}

#[derive(Clone)]
pub struct LogDb {
    mem: MemoryDb,
    log: LogFs,
    state: Arc<RwLock<State>>,
}

impl LogDb {
    pub fn log(&self) -> &LogFs {
        &self.log
    }

    pub fn open(path: impl Into<PathBuf>, key: String) -> Result<Self, AnyError> {
        let log = LogFs::open(path, key)?;

        let mem = MemoryDb::new();

        let mut counter = 1;
        for path in log.paths_prefix(b"_e/")? {
            let expected_path = Self::event_path(counter);

            let data = log
                .get(&path)?
                .ok_or_else(|| anyhow::anyhow!("Invalid Path: {:?}", path))?;
            let ev: DbEvent = serde_json::from_slice(&data)?;
            mem.apply_event(ev)?;

            counter += 1;
        }

        Ok(Self {
            log,
            mem,
            state: Arc::new(RwLock::new(State {
                event_index: counter,
            })),
        })
    }

    fn event_path(index: u64) -> Vec<u8> {
        let mut path = b"_e/".to_vec();
        path.extend(index.to_string().as_bytes());
        path
    }

    fn node_merge(&self, node: semantics_core::Node) -> Result<(), AnyError> {
        let mut state = self.state.write().unwrap();
        let node = self.mem.node_merge(node)?;
        self.persist_event(DbEvent::NodeMerged(node), &mut state)?;
        Ok(())
    }

    fn node_patch(&self, id: semantics_core::NodeId, data: Patch) -> Result<Node, AnyError> {
        let mut state = self.state.write().unwrap();
        // TODO: race condition.
        let node = self.mem.node_patch(id, data.clone())?;

        self.persist_event(DbEvent::NodeUpdated { id, data }, &mut state)?;

        Ok(node)
    }

    fn node_delete(&self, id: semantics_core::NodeId) -> Result<(), AnyError> {
        let mut state = self.state.write().unwrap();
        // TODO: race condition.
        self.mem.node_delete(id)?;
        self.persist_event(DbEvent::NodeDeleted { id }, &mut state)?;
        Ok(())
    }

    fn relation_merge(&self, relation: semantics_core::Relation) -> Result<(), AnyError> {
        let mut state = self.state.write().unwrap();
        self.persist_event(DbEvent::RelationMerged(relation.clone()), &mut state)?;
        self.mem.relation_merge(relation)?;
        Ok(())
    }

    fn relation_patch(
        &self,
        id: semantics_core::RelationId,
        data: Patch,
    ) -> Result<Relation, AnyError> {
        let mut state = self.state.write().unwrap();
        // TODO: race condition.
        let relation = self.mem.relation_patch(id, data.clone())?;
        self.persist_event(DbEvent::RelationUpdated { id, data }, &mut state)?;
        Ok(relation)
    }

    fn relation_delete(&self, id: semantics_core::RelationId) -> Result<(), AnyError> {
        let mut state = self.state.write().unwrap();
        // TODO: race condition.
        self.mem.relation_delete(id)?;
        self.persist_event(DbEvent::RelationDeleted { id }, &mut state)?;
        Ok(())
    }

    fn persist_event(&self, ev: DbEvent, state: &mut State) -> Result<(), AnyError> {
        let raw = serde_json::to_vec(&ev)?;

        state.event_index += 1;
        let path = Self::event_path(state.event_index);
        self.log.insert(path, raw)?;

        Ok(())
    }

    fn batch(&self, batch: Vec<DbEvent>) -> Result<(), AnyError> {
        let ev = DbEvent::Batch(batch);
        // TODO: race condition.
        let mut state = self.state.write().unwrap();
        self.mem.apply_event(ev.clone())?;
        self.persist_event(ev, &mut state)?;
        Ok(())
    }

    fn run_blocking<T, F>(&self, f: F) -> DbFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> Result<T, AnyError> + Send + 'static,
    {
        let s = self.clone();
        (async move {
            let out = spawn_blocking(move || f(&s)).await??;
            Ok(out)
        })
        .boxed()
    }
}

impl db::Db for LogDb {
    fn node(&self, id: semantics_core::NodeId) -> db::DbFuture<semantics_core::Node> {
        Db::node(&self.mem, id)
    }

    fn node_by_uri(&self, uri: String) -> DbFuture<Node> {
        self.run_blocking(move |s| s.mem.node_by_uri(uri))
    }

    fn nodes(
        &self,
        query: semantics_core::NodeQuery,
    ) -> db::DbFuture<semantics_core::Page<semantics_core::NodeItem>> {
        Db::nodes(&self.mem, query)
    }

    fn node_merge(&self, node: semantics_core::Node) -> db::DbFuture<()> {
        self.run_blocking(move |s| s.node_merge(node))
    }

    fn node_patch(
        &self,
        id: semantics_core::NodeId,
        data: Patch,
    ) -> db::DbFuture<semantics_core::Node> {
        self.run_blocking(move |s| s.node_patch(id, data))
    }

    fn node_delete(&self, id: semantics_core::NodeId) -> db::DbFuture<()> {
        self.run_blocking(move |s| s.node_delete(id))
    }

    fn relation(&self, id: semantics_core::RelationId) -> db::DbFuture<semantics_core::Relation> {
        Db::relation(&self.mem, id)
    }

    fn relations(
        &self,
        query: semantics_core::RelationQuery,
    ) -> db::DbFuture<semantics_core::Page<semantics_core::RelationItem>> {
        Db::relations(&self.mem, query)
    }

    fn relation_merge(&self, relation: semantics_core::Relation) -> db::DbFuture<()> {
        self.run_blocking(move |s| s.relation_merge(relation))
    }

    fn relation_patch(
        &self,
        id: semantics_core::RelationId,
        data: Patch,
    ) -> db::DbFuture<semantics_core::Relation> {
        self.run_blocking(move |s| s.relation_patch(id, data))
    }

    fn relation_delete(&self, id: semantics_core::RelationId) -> db::DbFuture<()> {
        self.run_blocking(move |s| s.relation_delete(id))
    }

    fn batch(&self, batch: Vec<DbEvent>) -> DbFuture<()> {
        self.run_blocking(move |s| s.batch(batch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_logdb() {
        let path = std::env::temp_dir().join("semantics_tests/logdb/full.log");
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        let db = LogDb::open(path, "test".into()).unwrap();
        semantics_core::db::test_db_async(db).await;
    }
}
