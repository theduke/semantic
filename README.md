# Semantic

This project has moved to [semantiverse/semantic](https://github.com/semantiverse/semantic).
This repository is archived. Continued development takes place in the new repository.

The consolidated repository contains all three versions:

- `v1`: the original implementation from this repository.
- `v2`: the intermediate implementation.
- `main`: the new implementation, previously on this repository's `ng` branch.

## Development

The project provides helper scripts for common tasks:

* `cargo xtask run`
  Run a development server that also auto-rebuilds the wasm UI.
* `cargo xtask install`
  Install the `semantic` binary locally.
* `cargo xtask build-ui`
  Build the wasm UI in release mode
