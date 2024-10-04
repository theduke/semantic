//! Semantic CLI binary.

// Enable async fn in traits.
#![allow(incomplete_features)]

mod cmd;

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";

        #[cfg(debug_assertions)]
        let default =
            "semantic=trace,logfs=trace,factordb=debug,factor_engine=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }

    // Initialize logger.
    // TODO: tracing-tree disabled until it supports tracing_subscriber 0.3
    // let subscriber =
    //     tracing_subscriber::Registry::default().with(tracing_tree::HierarchicalLayer::new(2));
    // tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing_subscriber::fmt::init();

    let args = <cmd::Args as clap::Parser>::parse();

    args.run().unwrap();
}
