#[cfg(all(feature = "client-http-native", not(target_arch = "wasm32")))]
#[path = "http_client.rs"]
pub mod http_client;

#[cfg(all(feature = "client-http-web", target_arch = "wasm32"))]
#[path = "http_client_wasm.rs"]
pub mod http_client;

#[cfg(all(feature = "client-ws-native", not(target_arch = "wasm32")))]
#[path = "ws_client_native.rs"]
pub mod ws_client;

#[cfg(all(feature = "client-ws-web", target_arch = "wasm32"))]
#[path = "ws_client_wasm.rs"]
pub mod ws_client;
