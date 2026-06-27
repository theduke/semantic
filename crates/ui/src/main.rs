#[cfg(feature = "desktop")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rpc_url = arg_value(&args, "--rpc-url")
        .or_else(|| std::env::var("SEMANTIC_RPC_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8888/api/v1/rpc".to_string());

    if arg_flag(&args, "--standalone") {
        #[cfg(feature = "standalone")]
        {
            launch_standalone(&args);
            return;
        }
        #[cfg(not(feature = "standalone"))]
        {
            eprintln!("standalone mode requires the 'standalone' feature");
            std::process::exit(2);
        }
    }

    let client = semantic_rpc::transport::http_client::HttpRpcClient::new(rpc_url).into();
    semantic_ui::launch_with_client(client, Some("default".to_string()));
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn launch_standalone(args: &[String]) {
    let app_config = app_config(args);
    let db_path = arg_value(args, "--db")
        .map(Into::into)
        .unwrap_or_else(|| app_config.default_db_path());
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).expect("create semantic UI database directory");
    }
    let blob_uri = app_config
        .default_blob_uri()
        .expect("build default semantic UI blob store uri");
    let handle = match semantic_ui::build_embedded_handle_with_blob_store(db_path, blob_uri) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to open semantic database: {err}");
            std::process::exit(1);
        }
    };
    let file_api_prefix = "semantic-file://".to_string();
    let config = standalone_desktop_config(handle.clone());
    semantic_ui::launch_with_client_file_api_and_config(
        handle.client,
        Some(handle.scope_id.to_string()),
        Some(file_api_prefix),
        config,
    );
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn standalone_desktop_config(handle: semantic_ui::EmbeddedAppHandle) -> dioxus::desktop::Config {
    use std::borrow::Cow;

    dioxus::desktop::Config::new().with_custom_protocol("semantic-file", move |_id, request| {
        let response = futures::executor::block_on(read_standalone_file(&handle, request.uri()));
        match response {
            Ok((mime_type, bytes)) => dioxus::desktop::wry::http::Response::builder()
                .status(200)
                .header("content-type", mime_type)
                .header("content-length", bytes.len().to_string())
                .body(Cow::Owned(bytes))
                .expect("valid semantic file response"),
            Err((status, message)) => dioxus::desktop::wry::http::Response::builder()
                .status(status)
                .header("content-type", "text/plain; charset=utf-8")
                .body(Cow::Owned(message.into_bytes()))
                .expect("valid semantic file error response"),
        }
    })
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
async fn read_standalone_file(
    handle: &semantic_ui::EmbeddedAppHandle,
    uri: &dioxus::desktop::wry::http::Uri,
) -> std::result::Result<(String, Vec<u8>), (u16, String)> {
    use futures::StreamExt as _;

    let id = file_id_from_custom_uri(uri)
        .ok_or_else(|| (400, "semantic-file URL must contain a file id".to_string()))?;
    let ctx = semantic_app::AppRequestContext {
        app: handle.app.clone(),
        principal: handle.principal.clone(),
        session: Some(std::sync::Arc::clone(&handle.session)),
        request_scope: Some(handle.scope_id.clone()),
    };
    let mut file = handle
        .app
        .files()
        .read(&ctx, Some(handle.scope_id.clone()), id)
        .await
        .map_err(|err| (404, err.to_string()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = file.stream.next().await {
        let chunk = chunk.map_err(|err| (500, err.to_string()))?;
        bytes.extend_from_slice(&chunk);
    }
    Ok((
        file.mime_type
            .take()
            .unwrap_or_else(|| "application/octet-stream".to_string()),
        bytes,
    ))
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn file_id_from_custom_uri(uri: &dioxus::desktop::wry::http::Uri) -> Option<String> {
    let raw = uri
        .host()
        .filter(|host| !host.is_empty())
        .or_else(|| uri.path().trim_start_matches('/').split('/').next())
        .filter(|value| !value.is_empty())?;
    percent_decode(raw)
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        match bytes[idx] {
            b'%' if idx + 2 < bytes.len() => {
                let hi = hex_value(bytes[idx + 1])?;
                let lo = hex_value(bytes[idx + 2])?;
                out.push((hi << 4) | lo);
                idx += 3;
            }
            byte => {
                out.push(byte);
                idx += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(all(feature = "desktop", feature = "standalone"))]
fn app_config(args: &[String]) -> semantic_app::AppConfig {
    let mut config = semantic_app::AppConfig::from_env();
    if let Some(data_dir) = arg_value(args, "--data-dir") {
        config.data_dir = Some(data_dir.into());
    }
    config
}

#[cfg(all(not(feature = "desktop"), target_arch = "wasm32"))]
fn main() {
    semantic_ui::app::launch_web();
}

#[cfg(all(not(feature = "desktop"), not(target_arch = "wasm32")))]
fn main() {
    eprintln!("semantic_ui binary currently requires the 'desktop' or 'web' Dioxus target");
}

#[cfg(feature = "desktop")]
fn arg_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

#[cfg(feature = "desktop")]
fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}
