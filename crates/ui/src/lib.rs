pub mod app;
pub mod backend;
pub mod screens;

pub use app::{AppRoot, AppRootProps, launch_with_client};
pub use backend::EmbeddedRpcClient;
#[cfg(any(feature = "desktop", feature = "server"))]
pub use backend::build_embedded_client;
