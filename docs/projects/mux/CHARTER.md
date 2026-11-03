# rusty_mux charter

Status: active · Owner: Nano · Started: 2026-10-22 · Name confirmed by the owner 2026-10-22.

## Mission

A server-side terminal multiplexer with every Rex capability ([CAPABILITIES.md](CAPABILITIES.md), the 28 rows of the design brief §3), built our own way: Rust, first-party crates, no hidden egress, Windows included.

Design brief: `REX-PARITY-DESIGN.md` (owner upload, not in the repo). Adopt/build evidence: [MUX-ADOPT-DECISIONS.md](../../research/MUX-ADOPT-DECISIONS.md). Engine backlog: [RUSTY-TERM-PERF-BACKLOG.md](../../research/RUSTY-TERM-PERF-BACKLOG.md). rusty_term project docs: [../term/](../term/README.md).

## Non-goals

- Rex compatibility (wire, CLI, API, config, Lua) and tmux compatibility.
- Forking phux. phux is read and its ideas re-written, not copied. Rex is known only from its public docs, as summarized in the brief.
- Any Nexus integration. Nexus is frozen and is a read-only reference.

## Constraints

- **Rules.** Small, terse, modular, ports-and-adapters; composition over inheritance; no abstraction before two real call sites. No `unwrap`/`expect` in production code; `Result` + `?`. Async only for real I/O. Tests for all non-trivial logic, failure and boundary cases included. Docstrings on public surface. Never swallow errors. Validate input at boundaries. No secrets in code or logs.
- **Reuse over rebuild.** Check existing crates first. No second TLS, HTTP, JSON, MCP or crypto implementation.
- **Sovereignty.** No hidden egress. libghostty-vt is a comparison oracle only, never in the runtime path. Audit anything adopted for outbound connections.
- **Layers.** ADR-0003: no app-to-app edges. ADR-0002: dependency sovereignty policy. `crates/libs/net/rusty_tls` is not edited.
- **Monorepo hygiene.** PRs scoped to their own crate paths; small PRs; merge commits through the merge queue; no force-push; never merge on red CI.
- **Evidence.** Every claim in these docs cites a file read or a command run; anything else is marked "unverified".
- **Owner approval required** for: new crates or names, moving `sessionmgr-proc` to libs, converging the two ConPTY wrappers, any new third-party dependency, any deletion, any change to `rusty_tls`, starting Phase 2.

## Definition of done, per phase

A phase is done when every exit criterion in [ROADMAP.md](ROADMAP.md) has evidence (a merged PR or a CI test), its capability rows are `done`, the fidelity and benchmark gates are green, and the owner has accepted a go/no-go summary. The next phase starts only after that acceptance.
