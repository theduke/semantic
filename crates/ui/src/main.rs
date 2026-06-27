#[cfg(feature = "desktop")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rpc_url = arg_value(&args, "--rpc-url")
        .or_else(|| std::env::var("SEMANTIC_RPC_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8888/rpc".to_string());

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
    let (client, scope_id) = match semantic_ui::build_embedded_client(db_path) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to open semantic database: {err}");
            std::process::exit(1);
        }
    };
    semantic_ui::launch_with_client(client, Some(scope_id));
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
