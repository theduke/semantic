pub mod client;
#[cfg(feature = "client")]
pub mod file;
pub mod interface;
#[cfg(feature = "interface-session")]
pub mod plugin;
pub mod registry;

#[cfg(feature = "client")]
pub mod transport;

#[cfg(any(feature = "server-axum", feature = "server-axum-ws"))]
pub mod server;

#[cfg(feature = "client")]
pub use client::{RpcClient, RpcClientDyn};
pub use registry::RpcRegistry;
