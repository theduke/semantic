# Architecture

This repository is split into small crates that separate data modeling, storage,
application coordination, and transport concerns.

## Data and Schema Layer

`semantic_data` defines the shared value, expression, query, and schema data
types used across the system. It is the neutral contract layer: database crates,
RPC payloads, CLI code, and application code all depend on these public data
structures rather than on a concrete backend.

## Database Layer

`semantic_db_core` defines the core database abstractions, query model,
catalog, validation, planning, and the `Db` wrapper around backend
implementations. Concrete backend crates include `semantic_db_kv`,
`semantic_db_redb`, and `semantic_db_postgres`; test helpers live in
`semantic_db_test`, and `semantic_db_cli` provides command-line access.

The federation engine in `semantic_db_core` plans reads over `QuerySource`
collections using an in-memory overlay catalog. Each source negotiates its
filter, ordering and pagination guarantees; the shared executor applies the
remaining query operations and combines local and virtual rows. Overlay schema
definitions and collections never enter the persisted database catalog.

## App Layer

`semantic_app` is the standalone application layer. It owns principals,
sessions, request contexts, database scope management, provider-based database
opening, command registration, and built-in app/database commands. It is
transport-independent and can be used directly by non-RPC frontends or tools.

`semantic_vdb` supplies the plugin-author `VirtualDatabase` API and its
`semantic.vdb/v1` interface adapter, plugin-bound query sources, row validation,
and per-scope runtime schema cache. The app routes reads that reference a virtual
collection through federation while local-only requests retain their original
path. Virtual collections are read-only, resolved lazily from plugin activations,
and surfaced through separate VDB list, schema and explain commands. See the
[plugin virtual database guide](plugin-virtual-databases.md).

## RPC Layer

`semantic_rpc` defines the RPC protocol, request/response types, command traits,
command registry, conversion helpers, and optional client/server transport
adapters. It provides the command invocation contract without owning application
state or database scope policy.

## Server Layer

`semantic_server` exposes `semantic_app` over axum. It wires HTTP and websocket
RPC endpoints, request/connection scope resolution, principal resolution, and
the default local redb flow. It should stay focused on transport concerns and
delegate app behavior to `semantic_app`.

## UI Layer

`semantic_ui_core` defines reusable Dioxus UI foundations: Dioxus context
helpers for the shared `semantic_rpc::RpcClient`, active scope context, a
schema-aware UI catalog, renderer registries, media/menu extension points, and
generic value/object/class components. The UI catalog is loaded through the app
catalog RPC command and then provided as loaded context, so normal rendering
components do not have to handle asynchronous catalog availability.

`semantic_ui` is the concrete UI application. It wires RPC clients, catalog
loading, generic catalog/collection/entity/query screens, and standalone modes
on top of `semantic_ui_core`. Embedded desktop mode uses an app-backed
`semantic_rpc::RpcClient` adapter, while server-backed mode can use the RPC
transport clients from `semantic_rpc`.
