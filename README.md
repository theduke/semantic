# Semantic

## Development

* Start backend server:
  `cargo run`
* Start UI development server:
  ```
  cd semantic_ui
  RUSTFLAGS="" CARGO_TARGET_DIR=../target_wasm trunk serve --release --dist ../target_wasm/ui
  ```
