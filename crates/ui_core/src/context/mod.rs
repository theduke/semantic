mod rpc_client;
mod scope;

pub use rpc_client::{provide_rpc_client, use_rpc_client};
pub use scope::{
    UiScopeContext, provide_ui_scope_context, use_active_scope_id, use_ui_scope_context,
};
