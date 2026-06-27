pub mod app;
#[cfg(feature = "standalone")]
pub mod backend;
pub mod screens;

pub use app::{AppRoot, AppRootProps, launch_with_client};
#[cfg(feature = "standalone")]
pub use backend::EmbeddedRpcClient;
#[cfg(feature = "standalone")]
pub use backend::build_embedded_client;
