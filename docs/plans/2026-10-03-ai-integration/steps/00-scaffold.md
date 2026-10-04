# Step 00: Scaffold workspace

Wave W0. Depends on: owner confirmation of plan decisions 3-5. Agent: opus.

## Goal

Create every new crate as a compiling skeleton and add all third-party workspace
dependencies once, so that parallel agents in later waves rarely touch
`Cargo.toml`/`Cargo.lock`.

## Scope

1. Create crates (workspace members are `crates/*`, so no root member edit is needed):

   | Path | Package name | lib.rs doc summary |
   |---|---|---|
   | `crates/agent` | `semantic_agent` | Protocol-neutral coding-agent model, events, capabilities and driver traits. |
   | `crates/agent_drivers` | `semantic_agent_drivers` | Process supervisor and concrete drivers (Claude Code, Codex, ACP, fake). |
   | `crates/agent_vcs` | `semantic_agent_vcs` | Workspace path safety and git checkpoints, diffs, worktrees. |
   | `crates/agent_domain` | `semantic_agent_domain` | `semantic.agents` package, RPC contract and thread event model. |
   | `crates/agent_orchestrator` | `semantic_agent_orchestrator` | Agent runtime: thread actors, persistence, live updates, recovery. |
   | `crates/agent_mcp` | `semantic_agent_mcp` | MCP server exposed to running agents. |
   | `crates/ai_ui` | `semantic_ai_ui` | Reusable Dioxus AI chat and coding-agent UI. |

   Each `Cargo.toml` uses the `*.workspace = true` package fields, like `crates/jobs/Cargo.toml`.
   Each `lib.rs` contains only the crate-level doc comment (`//!`) describing the
   responsibility and the dependency rules from plan.md. Do not create placeholder
   modules; later steps create their modules.

2. Declare inter-crate dependencies according to the crate map in plan.md (path deps).
   Use `semantic_data = { workspace = true }`.

3. Feature flags:
   * `semantic_agent_drivers`: `default = ["claude", "codex", "acp"]`, plus `claude`,
     `codex`, `acp` and `testkit` (test helpers, also enabled as a dev-dependency
     of downstream crates). The `fake` driver is always compiled; it is tiny and
     needed by orchestrator tests and e2e.
   * `semantic_ai_ui`: `default = ["markdown"]`, `markdown`, `web`, `desktop`,
     mirroring `semantic_ui_core`'s pattern. `agents` stays non-optional, because
     the chat module alone would still need the domain types for the RPC source;
     revisit if a non-agent consumer appears.

4. Add third-party dependencies to `[workspace.dependencies]` in the root
   `Cargo.toml` and reference them from the crates that need them. Pin the current
   latest compatible versions, checking crates.io at implementation time:
   * `rmcp` with features `server`, `macros`, `schemars`,
     `transport-streamable-http-server`, `transport-io`; dev: `client`,
     `transport-streamable-http-client-reqwest`.
   * `schemars` (the version rmcp requires).
   * `agent-client-protocol-schema` (1.x, v1 types).
   * `serde`/`serde_json` (already present), `tokio` (present, add features per
     crate), `tokio-util`, `tokio-stream`, `futures`, `async-trait` only if
     unavoidable (prefer `BoxFuture`), `thiserror`, `tracing`, `sha2`, `hex`,
     `uuid` (v4), `time`, `bytes`, `axum` (present), `reqwest` (MCP stdio bridge
     HTTP client, `rustls`, no default features), `which` (binary discovery),
     `nix` or `rustix` (process groups and signals on unix; pick one, prefer
     `rustix` if already in `Cargo.lock`), `tempfile` (dev).
   * Workspace-local path dependencies for the new crates, like `semantic_data`.

5. `crates/ai_ui` follows `crates/ui_core/Cargo.toml`:
   * dioxus with the router feature;
   * dxcomp, dxeditor (markdown, with the target-specific `web` feature as
     ui_core does), dioxus-sdk-time, dioxus-icons, futures;
   * `semantic_ui_core` (default-features = false), `semantic_rpc` (`client`);
   * dev-deps `dioxus-ssr` and `dioxus-html` (serialize).

6. Verify:
   * `nix develop -c cargo check --quiet --message-format=short` for the workspace;
   * `nix develop -c cargo check --target wasm32-unknown-unknown -p semantic_agent -p semantic_agent_domain -p semantic_ai_ui --features semantic_ai_ui/web`.
   If the wasm target or toolchain is missing in the devshell, record that and
   check how `crates/ui` web builds are verified (`dx build --web`).

## Out of scope

Any types or logic. Even helper modules belong to later steps.

## Acceptance

* All seven crates exist and compile (native).
* `semantic_agent`, `semantic_agent_domain` and `semantic_ai_ui` compile for
  wasm32, or the documented equivalent web check passes.
* Root `Cargo.toml` contains the new workspace deps; `Cargo.lock` is updated.
* One commit: "Scaffold agent integration crates".
