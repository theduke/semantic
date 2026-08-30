build-release:
    dx build --web --release --package semantic_ui --no-default-features --features web --debug-symbols=false --locked
    cargo build --release --package semantic_cli --bin semantic --features embed-ui --locked
