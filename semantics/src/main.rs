use factordb::schema::AttributeDescriptor;
use semantics_core::plugin::PluginDescriptor;

mod app;
mod blobstore;
mod db;
mod server;

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "semantics=trace");
    }
    tracing_subscriber::fmt::init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();

    match args.get(0).map(|x| x.as_str()).unwrap_or("server") {
        "generate-ts-base" => {
            let mut schema = semantics_core::base::SemanticPlugin::schema().db;
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
                    std::env::home_dir()
                        .expect("Could not determine home dir")
                        .join(".local/share/semantics")
                        .to_str()
                        .expect("invalid data path")
                        .to_string()
                });

                Some(semantics_core::api::BackendConfig::Crypto {
                    data_path,
                    key: "hello".into(),
                })
            };

            let config = app::AppConfig {
                backend: backend_config,
                token_key: "tokens".into(),
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            rt.block_on(server::run_server(app));
        }
        #[cfg(feature = "webkit")]
        "webkit" => {
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
