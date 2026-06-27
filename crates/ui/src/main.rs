#[cfg(feature = "desktop")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "serve") {
        #[cfg(feature = "server")]
        {
            run_server(&args);
            return;
        }
        #[cfg(not(feature = "server"))]
        {
            eprintln!("server mode requires the 'server' feature");
            std::process::exit(2);
        }
    }

    let db_path = arg_value(&args, "--db").unwrap_or_else(|| "semantic.redb".to_string());
    let (client, scope_id) = match semantic_ui::build_embedded_client(db_path) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to open semantic database: {err}");
            std::process::exit(1);
        }
    };
    semantic_ui::launch_with_client(client, Some(scope_id));
}

#[cfg(all(feature = "server", feature = "desktop"))]
fn run_server(args: &[String]) {
    let db_path = arg_value(args, "--db").unwrap_or_else(|| "semantic.redb".to_string());
    let bind = arg_value(args, "--bind").unwrap_or_else(|| "127.0.0.1:3000".to_string());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let server = semantic_server::SemanticServer::local_redb(db_path)
            .expect("local redb semantic server");
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .expect("bind semantic UI server");
        eprintln!("semantic UI server listening on http://{bind}");
        server.serve(listener).await.expect("serve semantic UI");
    });
}

#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!("semantic_ui binary currently requires the 'desktop' feature");
}

#[cfg(feature = "desktop")]
fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}
