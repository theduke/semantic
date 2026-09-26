//! Configuration of [`RedbKvEngine`](crate::RedbKvEngine).

/// Default page cache size: 256 MiB.
pub const DEFAULT_CACHE_SIZE: usize = 256 * 1024 * 1024;

/// Durability of committed write transactions (see [`redb::Durability`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RedbDurability {
    /// Commits are persisted (fsynced) before the commit returns.
    #[default]
    Immediate,
    /// Commits are written without waiting for them to be persisted; a
    /// crash or power loss may lose the latest commits.
    Eventual,
    /// Commits are only persisted by a later durable commit. The engine
    /// issues one when it is flushed or dropped, so a crash loses every
    /// commit since then. Intended for tests and throwaway databases.
    None,
}

impl From<RedbDurability> for redb::Durability {
    fn from(value: RedbDurability) -> Self {
        match value {
            RedbDurability::Immediate => Self::Immediate,
            RedbDurability::Eventual => Self::Eventual,
            RedbDurability::None => Self::None,
        }
    }
}

/// Options of [`RedbKvEngine`](crate::RedbKvEngine).
///
/// The default uses durable commits and a [`DEFAULT_CACHE_SIZE`] page cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedbOptions {
    /// Memory used by redb for caching pages (read cache and write buffer),
    /// in bytes.
    pub cache_size: usize,
    /// Durability of write transactions committed through the engine.
    pub durability: RedbDurability,
    /// Persist allocator state with every commit so that recovery after a
    /// crash is almost instant, at the cost of slower commits.
    pub quick_repair: bool,
}

impl Default for RedbOptions {
    fn default() -> Self {
        Self {
            cache_size: DEFAULT_CACHE_SIZE,
            durability: RedbDurability::default(),
            quick_repair: false,
        }
    }
}

impl RedbOptions {
    pub fn with_cache_size(mut self, bytes: usize) -> Self {
        self.cache_size = bytes;
        self
    }

    pub fn with_durability(mut self, durability: RedbDurability) -> Self {
        self.durability = durability;
        self
    }

    pub fn with_quick_repair(mut self, enabled: bool) -> Self {
        self.quick_repair = enabled;
        self
    }

    pub(crate) fn builder(&self) -> redb::Builder {
        let mut builder = redb::Builder::new();
        builder.set_cache_size(self.cache_size);
        builder
    }
}
