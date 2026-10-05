# AGENTS.md

Context for every agent working in this repo (Claude Code, Codex, local models). Single source of truth; `CLAUDE.md` imports it.

## What this is
rusty_orch: a multi-model orchestrator built on a shared blackboard instead of agent messaging. Read ARCHITECTURE.md and docs/adr/ before changing structure.

## Rules
- `crates/orch-core` stays pure: no I/O, no async, no dependencies. Adapters go in new crates under `crates/`.
- Rust: `Result` + `?` in library code; no `unwrap`/`expect` outside tests. Make illegal states unrepresentable. Flat control flow, short single-purpose functions, docstrings on public items.
- No abstraction before two real call sites. No new third-party dependency without a stated justification in the PR.
- Tests for all non-trivial logic: happy path plus failure and boundary cases.
- Never commit or log secrets. No vendor API keys: CLIs authenticate with their own subscription logins.

## Workflow
- Every change lands through a PR to `main`; merge with a merge commit after green CI (fmt, clippy `-D warnings`, test).
- Update both RELEASE_NOTES.md and CHANGELOG.md for any meaningful change.
- Record non-obvious decisions as a new ADR in docs/adr/.
- Ask before hard-to-reverse changes: public API breaks in `orch-core`, deletions, toolchain bumps.
