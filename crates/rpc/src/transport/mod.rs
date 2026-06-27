#[cfg(feature = "client-http")]
pub mod http_client;

#[cfg(all(feature = "client-ws", not(target_arch = "wasm32")))]
#[path = "ws_client_native.rs"]
pub mod ws_client;

#[cfg(all(feature = "client-ws", target_arch = "wasm32"))]
#[path = "ws_client_wasm.rs"]
pub mod ws_client;
