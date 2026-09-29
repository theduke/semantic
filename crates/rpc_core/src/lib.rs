//! Transport-independent RPC contracts and runtime packages.
pub mod command;
pub mod error;
pub mod interface;
pub mod interface_protocol;
pub mod package;
pub mod protocol;

pub use command::{
    AttrScopeId, CallError, CommandAdapter, CommandDef, DynCommand, RpcCommand, RpcCommandSpec,
};
pub use error::{RegisterError, RpcClientError, RpcError};
pub use package::RuntimePackage;
pub use protocol::{RpcRequest, RpcRequestId, RpcResponse, RpcResult};
