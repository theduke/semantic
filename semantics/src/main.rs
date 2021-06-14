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

    let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");

    let mode = std::env::var("SEMANTICS_MODE").unwrap_or("server".to_string());

    let app = app::App::build(config, rt.handle().clone()).expect("Could not build app");

    match mode.as_str() {
        "server" => {
            rt.block_on(server::run_server(app));
        }
        "gtk" => {
            app.run_webview_gtk().expect("Could not run GTK app");
        }
        other => {
            eprintln!("Unknown mode: '{}'", other);
            std::process::exit(1);
        }
    }
}
