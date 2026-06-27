pub use semantic_db_core::AsyncRuntime as KvBackendSpawner;
pub use semantic_db_core::InlineAsyncRuntime as InlineSpawner;

#[cfg(feature = "tokio")]
pub use semantic_db_core::TokioAsyncRuntime as TokioSpawner;

#[cfg(feature = "tokio")]
pub type DefaultKvBackendSpawner = TokioSpawner;

#[cfg(not(feature = "tokio"))]
pub type DefaultKvBackendSpawner = InlineSpawner;
