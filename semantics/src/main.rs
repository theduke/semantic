use factordb::schema::AttributeDescriptor;
use semantics_core::PluginDescriptor;

mod app;
mod blobstore;
mod db;
mod server;

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "semantics=trace");
    }
    tracing_subscriber::fmt::init();

    let data_path = std::env::home_dir()
        .expect("Could not determine home dir")
        .join(".local/share/semantics");
    let config = app::AppConfig {
        data_path,
        key: "hello".into(),
    };

    let cmd = std::env::args().skip(1).collect::<Vec<_>>();

    match cmd.get(0).map(|x| x.as_str()).unwrap_or("server") {
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
