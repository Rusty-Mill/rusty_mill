# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `orch-ollama` crate: `Agent::Local` over the Ollama CLI. `render` (card + referenced live entries + format spec), strict `parse` (kinds per role, confidence, ref syntax, caps), `OllamaAgent` over a `CommandRunner` seam with `StdCommand` (stdin thread, deadline, kill and reap, 1 MiB stdout cap). `research` example and an ignored real-ollama test. ADR-0004 (output protocol).
- `orch-dispatch` crate: `AgentRunner` port, validated `Routing` (work roles → one agent; reviews → ordered preference list, first agent ≠ author), sequential `Dispatcher` loop over `Plan` with goal and per-task call ceilings in a caller-owned `Ledger`, atomic per-call board writes, retry exhaustion → `Plan::fail`, `FakeAgent` behind the `fake` feature. ADR-0003 (dispatcher reuse boundary).
- `orch-core` domain crate: goal contracts, task cards with `Plan` lifecycle, append-only blackboard.
- Repo governance set, Rust CI, `AGENTS.md`/`CLAUDE.md`, ADR-0001 and ADR-0002.
### Fixed
- `SECURITY.md` advisory link points at `Rusty-Mill/rusty_mill` instead of the retired standalone repo.
