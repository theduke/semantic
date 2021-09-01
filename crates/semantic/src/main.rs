use factordb::schema::AttributeDescriptor;
use semantic_core::plugin::PluginDescriptor;

mod app;
mod blobstore;
mod db;
mod server;

fn build_config(args: &[String]) -> app::AppConfig {
    let no_backend = args.iter().any(|x| x == "--no-backend");

    let backend_config = if no_backend {
        None
    } else {
        let data_path_arg = args
            .iter()
            .find(|x| x.starts_with("--data-path"))
            .and_then(|x| x.split_once('='))
            .map(|x| x.1.to_string());

        let data_path = data_path_arg.unwrap_or_else(|| {
            dirs::home_dir()
                .expect("Could not determine home dir")
                .join(".local/share/semantics/db.data")
                .to_str()
                .expect("invalid data path")
                .to_string()
        });

        let key = args
            .iter()
            .find(|x| x.starts_with("--key="))
            .and_then(|x| x.split_once('='))
            .map(|x| x.1.to_string())
            // FIXME: obviously just for debugging, remove this.
            .unwrap_or("random key".to_string());

        Some(semantic_core::api::BackendConfig::Crypto {
            data_path: Some(data_path),
            key,
        })
    };

    app::AppConfig {
        backend: backend_config,
        token_key: "tokens".into(),
        server: None,
    }
}

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";
        #[cfg(debug_assertions)]
        let default = "semantic=trace,logfs=trace,factordb=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }
    tracing_subscriber::fmt::init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();

    match args.get(0).map(|x| x.as_str()).unwrap_or("server") {
        "generate-ts-base" => {
            let mut schema = semantic_core::base::SemanticPlugin::schema().db;
            // let builtin = factordb::schema::builtin::builtin_db_schema();
            schema
                .attributes
                .push(factordb::schema::builtin::AttrId::schema());
            schema
                .attributes
                .push(factordb::schema::builtin::AttrIdent::schema());

            let ts = factor_tools::typescript::schema_to_typescript(&schema, None)
                .expect("Could not generate typescript");
            println!("{}", ts);
        }
        "generate-ts-builtin" => {
            let schema = factordb::schema::builtin::builtin_db_schema();
            let ts = factor_tools::typescript::schema_to_typescript(&schema, None)
                .expect("Could not generate typescript");
            println!("{}", ts);
        }
        "server" => {
            let mut config = build_config(&args);
            config.server = Some(server::ServerConfig {
                interface: "127.0.0.1:3000".into(),
            });

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            rt.block_on(app.run_server()).expect("Server failed");
        }
        #[cfg(feature = "webkit")]
        "webkit" => {
            let config = build_config(&args);
            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            app.run_webview_gtk().expect("Could not run GTK app");
        }
        other => {
            eprintln!("Unknown command '{}'", other);
            std::process::exit(1);
        }
    }
}
