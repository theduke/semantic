//! Public server configuration exchanged with clients.

use semantic_data::Class;
use semantic_rpc_core::{RpcCommandSpec, RpcError};

/// Client-facing configuration, currently without configurable public fields.
///
/// This communication type is not registered in a schema bundle or migration.
#[derive(Class, Clone, Debug, Default, PartialEq, Eq)]
#[semantic(id = "semantic:server:config")]
pub struct ServerConfig {}

/// Retrieve the server's public configuration without selecting a database scope.
pub struct ConfigGet;

impl RpcCommandSpec for ConfigGet {
    type Payload = ();
    type Output = ServerConfig;
    type Error = RpcError;

    const NAME: &'static str = "semantic.server.config_get";
}
