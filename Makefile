

DATA_DIR ?= ./data
SEMANTIC_INTERFACE ?= 127.0.0.1
SEMANTIC_PORT ?= 8888

.PHONY: build-release server server-release ui-standalone ui-desktop ui-web

build-release:
	dx build --web --release --package semantic_ui --no-default-features --features web --debug-symbols=false --locked
	cargo build --release --package semantic_cli --bin semantic --features embed-ui --locked

server:
	mkdir -p $(DATA_DIR)
	SEMANTIC_DATA_DIR=$(DATA_DIR) SEMANTIC_INTERFACE=$(SEMANTIC_INTERFACE) SEMANTIC_PORT=$(SEMANTIC_PORT) cargo run -p semantic_server

server-release: build-release
	mkdir -p $(DATA_DIR)
	SEMANTIC_DATA_DIR=$(DATA_DIR) SEMANTIC_INTERFACE=$(SEMANTIC_INTERFACE) SEMANTIC_PORT=$(SEMANTIC_PORT) ./target/release/semantic

ui-standalone:
	mkdir -p $(DATA_DIR)
	SEMANTIC_DATA_DIR=$(DATA_DIR) dx serve --desktop --hot-patch --package semantic_ui --no-default-features --features standalone --args=--standalone

ui-desktop:
	dx serve --desktop --hot-patch --package semantic_ui --no-default-features --features desktop

ui-web:
	dx serve --web --package semantic_ui --no-default-features --features web

# Run a postgres server with docker.
# To connect, use user postgres, password postgres
run-postgres:
	docker run --name postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 -d postgres:alpine
