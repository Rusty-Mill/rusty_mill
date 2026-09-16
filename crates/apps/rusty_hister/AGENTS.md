# AGENTS.md

## Scope

Applies to `crates/rusty_hister/` (this cluster) within the RustyMill
workspace.

## Project shape

- Purpose: Rust port of [asciimoo/hister](https://github.com/asciimoo/hister)
  (AGPL-3.0-or-later), v1 scope backend-only (HTTP/JSON API + MCP JSON-RPC
  wire-compatible with Hister's existing frontends). See `README.md` for the
  crate list and `docs/PROJECT-STATUS.md` for current state.
- Rust structure: normal flat workspace members under
  `crates/rusty_hister/crates/rusty-hister-*`, registered directly in the
  root `Cargo.toml`'s `[workspace] members` — no nested `[workspace]`
  manifest (this cluster is fresh work, not a `git subtree` import of a
  pre-existing standalone repo, so there is no prior standalone-repo
  workspace file to preserve per ADR-0001's remit).
- Dependency direction: `rusty-hister-core` has no dependency on any other
  cluster crate; every other crate depends on `core` (and, where relevant,
  `model`) but not on each other's internals — `server` and `mcp` are the
  only crates that compose the others (indexer, extractor, crawler,
  vectorstore, model) into request handlers/tools.
- Source of truth for what must be ported: `docs/capability-inventory/
  HISTER-CAPABILITY-INVENTORY.md`. Every row is REQUIRED by default; check
  it, and this cluster's `docs/decisions/`, before assuming any capability
  is out of scope.

## Coordination

Follow `WORKFLOW.md` for PR/CI/merge mechanics — it governs process, not
project architecture.

## Canonical commands

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy -p rusty-hister-core -p rusty-hister-model -p
  rusty-hister-extractor -p rusty-hister-indexer -p rusty-hister-vectorstore
  -p rusty-hister-crawler -p rusty-hister-server -p rusty-hister-mcp
  --all-targets --all-features -- -D warnings`
- Test: same `-p` set with `cargo test`
- Docs/build: `cargo check` with the same `-p` set (the workspace is ~90
  crates; scope invocations to this cluster rather than `--workspace`,
  matching the root CI's own affected-crates-filter convention)

## Change rules

- `Result` + `?` over panics; `unwrap()`/`expect()` only inside
  `#[cfg(test)]`.
- Tests for all non-trivial logic: happy path plus at least one
  boundary/failure case. Where a Go `_test.go` exists for the capability
  being ported (capability-inventory marks each `[TESTED]`/`[UNTESTED]`),
  port its test *cases* by re-deriving fixtures from independent reading of
  the Go source and its behavior — do not copy Go source files (including
  `_test.go` files) verbatim into this MIT/Apache-2.0-licensed cluster; see
  ADR-0001's licensing note.
- Docstrings (`///`) on public items once a unit leaves bootstrap stage.
- No speculative generality: build against the capability inventory's
  actual rows, not a generalized abstraction for capabilities Hister
  doesn't have.
- Before adding any dependency (external or hand-rolled), check whether an
  existing `rusty_*` workspace crate already covers it — see ADR-0001's
  sovereignty-audit summary and its citations into the individual `rusty_*`
  crates' own docs. `rusty_tokio`, `rusty_http`/`rusty_request`, `rusty_tls`,
  `rusty_json`, `rusty_db`, `rusty_search`, `rusty_url`, `rusty_llama`/
  `rusty_provider`, and `rusty_mcp` are all confirmed fits for their
  respective needs — do not reach for an external crate or a hand-rolled
  replacement for what they already do.
- Flat control flow, guard clauses over deep nesting.
- Update `docs/PROJECT-STATUS.md` and `docs/roadmap/ROADMAP.md` when a
  roadmap unit's state changes.

## Definition of done

- Tests pass, lints clean, formatting clean (see Canonical commands).
- Public items documented.
- `docs/PROJECT-STATUS.md`/`docs/roadmap/ROADMAP.md` updated in the same PR
  (or an immediate follow-up) when a unit changes state.
- An ADR exists for any new consequential design choice — this project is in
  active bootstrap/major development, so default to writing one per delivery
  cycle (see `docs/decisions/`).
- No REQUIRED capability-inventory row is silently dropped. A row may move
  to out-of-scope only via an explicit, written, user-attributed sign-off
  recorded in an ADR — never inferred while implementing.
