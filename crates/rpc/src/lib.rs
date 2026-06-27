pub mod client;
pub mod command;
pub mod convert;
pub mod error;
pub mod protocol;
pub mod registry;

#[cfg(any(feature = "client-http", feature = "client-ws"))]
pub mod transport;

#[cfg(any(feature = "server-axum", feature = "server-axum-ws"))]
pub mod server;

pub use command::{CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec};
pub use convert::{RpcDecode, RpcEncode};
pub use error::{RegisterError, RpcClientError, RpcError};
pub use protocol::{RpcRequest, RpcRequestId, RpcResponse, RpcResult};
pub use registry::RpcRegistry;
