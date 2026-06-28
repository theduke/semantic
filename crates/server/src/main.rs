fn main() {
    let args: Vec<String> = std::env::args().collect();
    let app_config = app_config(&args);
    let server_config = match server_config(&args) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };
    let db_path = arg_value(&args, "--db")
        .map(Into::into)
        .unwrap_or_else(|| app_config.default_db_path());
    let blob_uri = match arg_value(&args, "--blob-uri") {
        Some(uri) => uri,
        None => app_config
            .default_blob_uri()
            .expect("build default semantic blob store uri"),
    };
    let bind = arg_value(&args, "--bind").unwrap_or_else(|| server_config.bind_address());
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).expect("create semantic server database directory");
    }

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let server = semantic_server::SemanticServer::local_redb_with_app_config_and_blob_store(
            db_path, app_config, blob_uri,
        )
        .expect("local redb semantic server")
        .with_config(server_config);
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .expect("bind semantic server");
        eprintln!("semantic server listening on http://{bind}");
        server.serve(listener).await.expect("serve semantic server");
    });
}

fn app_config(args: &[String]) -> semantic_app::AppConfig {
    let mut config = semantic_app::AppConfig::from_env();
    if let Some(data_dir) = arg_value(args, "--data-dir") {
        config.data_dir = Some(data_dir.into());
    }
    if let Some(temp_dir) = arg_value(args, "--temp-dir") {
        config.temp_dir = Some(temp_dir.into());
    }
    if arg_flag(args, "--auto-analyze-media") {
        config.auto_analyze_media = true;
    }
    config
}

fn server_config(args: &[String]) -> std::result::Result<semantic_server::ServerConfig, String> {
    let mut config = semantic_server::ServerConfig::from_env()?;
    if let Some(interface) = arg_value(args, "--interface") {
        config.interface = interface;
    }
    if let Some(port) = arg_value(args, "--port") {
        config.port = port
            .parse()
            .map_err(|err| format!("invalid --port: {err}"))?;
    }
    Ok(config)
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}

fn arg_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}
