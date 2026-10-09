# Session prompt: MCP track (replace `rmcp` with first-party crates)

You are working in the `rusty_mill` monorepo (Cargo workspace, Rust). Start from branch `claude/peaceful-dirac-200syz` (head contains all work described here); create your own branch from it and push only there. Do not open a PR unless asked.

## Read first
1. `docs/research/MCP-NATIVE-PLAN.md` (the plan, the two tracks, the open decisions).
2. `docs/adr/0002-*.md` (dependency sovereignty tiers: S sovereign, T transitional, A adapter).
3. `docs/research/CONSOLIDATION-AUDIT.md`, "Status" section and rows 11 and 12.
4. `crates/libs/protocol/rusty_agui` as the precedent: a native protocol crate on `rusty_json` with an optional `rusty_serve` layer.

## Goal
Replace `rmcp` and the stack under `rusty-mcp` (`axum`, `tokio`, `reqwest`, `clap`, `tracing-subscriber`, `jsonwebtoken`) with first-party crates: `rusty_json`, `rusty_tokio`, `rusty_http`, `rusty_serve`, `rusty_request`, `rusty_oauth`, `rusty_tls`.

## The rule you must keep
MCP crates depend on `rusty_request` / `rusty_serve` / `rusty_tls` and never on `rustls`, `reqwest` or `native-tls` directly. Another session is making `rusty_tls`'s native engine the default (the TLS track). You work against `rusty_tls` exactly as it is today and must not edit `crates/libs/net/rusty_tls`.

## State
Done: `rusty-mcp-client` split out of `rusty-mcp` (server scaffold only); `rk-mcp` and `rk-app` already on them; client on rustls, not OpenSSL. Open: A0 (ADR-0002 amendment, Tier A to T) through A6 in the plan.

## Decisions you do NOT make alone (ask the owner, one short question)
ADR-0002 amendment; tool schemas (builder first vs derive); blocking thread-per-connection servers on `rusty_serve`; scope (rmcp only vs its whole stack); anything that deletes code or changes a public API of a shared crate. Nexus (`crates/apps/nexus`) is frozen: do not touch or delete it.

## First task (needs no decision)
Read-only scoping for A2: inventory every `rmcp::model`, `rmcp::service` and `rmcp::handler` type the non-Nexus crates actually use (`rusty-mcp`, `rusty-mcp-client`, `rusty-mcp-demo`, `adk-mcp`, `remind_me_remote`, `rp-mcp`, `agentgateway*`, `rusty_homelab_mcp`, `rk-*`). Produce the minimal type list `rusty_mcp_proto` must cover and the wire messages needed for initialize, tools, resources, prompts, completion, pagination, cancel/progress. Write it to `docs/research/MCP-PROTO-SCOPE.md`, then stop and report.

## Working rules
Small, modular, minimal dependencies; justify any new third-party dependency (the aim is fewer). Type hints/explicit types, `Result` + `?`, no `unwrap()` outside tests. Tests for all non-trivial logic, including failure cases. Keep diffs focused; match surrounding code.
Before each commit: `cargo fmt --all`; `cargo clippy -p <crates> --all-targets -- -D warnings`; `cargo test -p <crates>`; then
`cargo metadata --format-version=1 --all-features --locked > $S/m.json` and run `.github/scripts/check_workspace_deps.py`, `check_workspace_layers.py`, and `generate_workspace_map.py > docs/WORKSPACE-MAP.md`. Add a CHANGELOG.md entry and a dated RELEASE_NOTES.md section (match existing style). New crates need `[package.metadata.rusty_mill] layer = ...`, a root `Cargo.toml` member and workspace-dependency entry, and a README table row. Disk is limited: if a build fails with "no space", `rm -rf target/debug` and rebuild.
Report faithfully: say what you did not verify (for example HTTP paths without a live server).
