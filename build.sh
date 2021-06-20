#!/usr/bin/env bash

set -euxo pipefail

# cargo run generate-ts-builtin > ui/src/schema/factor_generated.ts
# cargo run generate-ts-base > ui/src/plugins/semantic/generated.ts

cargo build --target-dir target_wasm --package semantic_ui --target wasm32-unknown-unknown
