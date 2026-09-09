//! Transport-independent RPC contracts and runtime packages.
pub mod command;
pub mod convert;
pub mod error;
pub mod package;
pub mod protocol;

pub use command::{CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec};
pub use convert::{RpcDecode, RpcEncode};
pub use error::{RegisterError, RpcClientError, RpcError};
pub use package::RuntimePackage;
pub use protocol::{RpcRequest, RpcRequestId, RpcResponse, RpcResult};
