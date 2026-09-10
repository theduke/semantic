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
    let db_uri = arg_value(&args, "--db-uri").or_else(|| std::env::var("SEMANTIC_DB_URI").ok());
    let blob_uri =
        match arg_value(&args, "--blob-uri").or_else(|| std::env::var("SEMANTIC_BLOB_URI").ok()) {
            Some(uri) => uri,
            None => app_config
                .default_blob_uri()
                .expect("build default semantic blob store uri"),
        };
    let bind = arg_value(&args, "--bind").unwrap_or_else(|| server_config.bind_address());
    let db_uri = semantic_server::resolve_db_uri(db_uri, &blob_uri, &app_config)
        .expect("resolve database URI");
    let blob_password =
        semantic_server::prompt_blob_password(&blob_uri).expect("read logfs password");

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let server =
            semantic_server::SemanticServer::from_uris(db_uri, blob_uri, blob_password, app_config)
                .await
                .expect("open semantic server storage")
                .with_config(server_config);
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .expect("bind semantic server");
        eprintln!("semantic server listening on http://{bind}");
        server
            .serve_with_shutdown(listener, async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .expect("serve semantic server");
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
