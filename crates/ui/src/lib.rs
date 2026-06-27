pub mod app;
#[cfg(feature = "standalone")]
pub mod backend;
pub mod components;
pub mod views;

#[cfg(feature = "desktop")]
pub use app::launch_with_client_file_api_and_config;
pub use app::{AppRoot, AppRootProps, launch_with_client, launch_with_client_and_file_api};
#[cfg(feature = "standalone")]
pub use backend::{
    EmbeddedAppHandle, EmbeddedRpcClient, build_embedded_client,
    build_embedded_handle_with_blob_store,
};
