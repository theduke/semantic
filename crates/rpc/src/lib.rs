pub mod client;
#[cfg(feature = "client")]
pub mod file;
pub mod registry;

#[cfg(feature = "client")]
pub mod transport;

#[cfg(any(feature = "server-axum", feature = "server-axum-ws"))]
pub mod server;

#[cfg(feature = "client")]
pub use client::{RpcClient, RpcClientDyn};
pub use registry::RpcRegistry;
