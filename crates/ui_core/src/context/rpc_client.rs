use dioxus::prelude::{
    ReadableExt, Signal, WritableExt, use_context, use_context_provider, use_effect, use_reactive,
    use_signal,
};
use semantic_rpc::RpcClient;

#[derive(Clone, Copy)]
struct RpcClientContext {
    client: Signal<RpcClient>,
}

pub fn provide_rpc_client(client: RpcClient) -> RpcClient {
    let mut active_client = use_signal(|| client.clone());
    use_context_provider(|| RpcClientContext {
        client: active_client,
    });
    use_effect(use_reactive((&client,), move |(client,)| {
        active_client.set(client);
    }));
    client
}

pub fn use_rpc_client() -> RpcClient {
    use_context::<RpcClientContext>().client.read().clone()
}
