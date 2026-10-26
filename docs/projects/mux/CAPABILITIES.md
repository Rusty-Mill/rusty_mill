# Capabilities

The 28 Rex capabilities from the design brief §3, re-baselined against **our** work, not phux's scorecard. Status: `not-started` / `in-progress` / `done` / `blocked`. Phases are from [ROADMAP.md](ROADMAP.md).

Crate names below follow the brief §8 components with the `rusty_mux_*` prefix, confirmed by the owner 2026-10-22 ([DECISIONS.md](DECISIONS.md)).

## Engine prerequisites (credited)

| Piece | Status | Evidence |
|---|---|---|
| VT parser + grid (`rusty_term_core`) | done | `crates/libs/ui/rusty_term/core/src/` |
| Headless core, no tokio | done | #851 |
| Fidelity gate (18-case corpus) | done | #857; `core/tests/fidelity.rs`; 15/18 match the libghostty oracle, the other 3 are the deliberate DECCOLM divergence (`docs/research/rusty-term-perf/FIDELITY.md`) |
| BCE erase | done | #862 |
| Instruction-count benchmark gate | done | #872 |
| Per-code-point widths, mode 2027 | done | #894 (closes #855) |
| DECCOLM side effects, never a resize | done | #856 |
| Ring-buffer screen + recycled scrollback rows | in-progress | #930 merged (memmove 4.6%); MB/s floors not met, #889 stays open (DECISIONS) |
| Printable-ASCII fast path | done | #936 (backlog #4) |
| Snapshot serializer | done | #939: 2.09 MB for 200x50 + 10k lines (target ≤ 7.8 MB); encode 9.3-14.3 ms (target ≤ 11.2 ms, not met reliably) |

## Capability matrix

| Id | Capability | Phase | Owning crate | Status | Evidence |
|---|---|---|---|---|---|
| C01 | Persistent sessions (outlive clients) | 1 | `rusty_mux_server` | in-progress | blocks and their PTYs outlive client connections; the server is not yet a daemon |
| C02 | Multi-client attach, independent viewports | 1 | `rusty_mux_server` | in-progress | `server/tests/phase1.rs`: two clients converge on one PTY; viewports are client-side |
| C03 | Snapshot + ready + background history | 1 | `rusty_mux_server`, `rusty_term_core` | in-progress | negotiated RTCP v3 (run-encoded, 0.64 MB at 200x50 + 10k history) checkpoint chunks then ready then raw output; normal 10,000-row history, no lifetime-output cutoff (see [REPLAY.md](REPLAY.md)); reattach test in `server/tests/phase1.rs`; newest-first history streaming not started |
| C04 | CLI automation (`--json`, stable exit codes) | 2 | `rusty_mux_ctl` | in-progress | `rusty-mux-ctl`, `ctl/tests/ctl.rs`; exit codes as Rex (0/1/2/3 target/4 unreachable/130); targets by id, name, position, id suffix or path (`ctl/src/target.rs`); ids default from `$RUSTY_MUX_SESSION`/`WINDOW`/`BLOCK` |
| C05 | Resumable event stream | 2 | `rusty_mux_proto`, `rusty_mux_server` | in-progress | `Features::EVENT_SEQ`, `server/tests/events.rs` |
| C06 | Per-resource (block) methods | 2 | `rusty_mux_server` | in-progress | methods of a block type (`terminal.format`, `terminal.pwd`, `terminal.title`) through one generic `BlockCall`, checked against the catalog; also `input`, `resize`, `close-block` by block id; discovery with `list`; `server/tests/inspect.rs` |
| C07 | Agent / MCP integration | 2 | `rusty_mux_mcp` on `rusty_mcp_server` | in-progress | `rusty-mux-mcp` (stdio): one tool per catalog method, same schema as `api describe`, each call run as ctl runs it; `mcp/src/tests.rs`, `mcp/tests/stdio.rs` |
| C08 | Command palette + directory browse | 4 | `rusty_mux` clients | not-started | |
| C09 | Self-hosted, no account, no telemetry | 1 | all | not-started | |
| C10 | Native scrollback and selection | 1 | `rusty_term_core`, `rusty_mux_tui` | in-progress | engine rows above |
| C11 | Session model (session/window/block/client) | 1 | `rusty_mux_core` | in-progress | model, single-writer input lease and event queue in `crates/apps/rusty_mux/crates/core`; the server wires it to PTYs |
| C12 | Tabs and splits, client-native | 4 | `rusty_mux` clients | not-started | |
| C13 | Mosh-like feel (roaming, predictive echo) | 3 | `rusty_mux_transport` | not-started | spike 4 |
| C14 | Live sharing | 3 | `rusty_mux_auth`, `rusty_mux_server` | not-started | |
| C15 | Web client | 4 | `rusty_mux` web client | not-started | |
| C16 | Client matrix (TUI, web, native) | 4 | `rusty_mux` clients | not-started | |
| C17 | Self-describing API (schema-first) | 2 | `rusty_mux_proto` | in-progress | `schema::CATALOG`; `rusty-mux-ctl api list` and `api describe` (parameters as JSON Schema) |
| C18 | Screen capture | 2 | `rusty_mux_server` | in-progress | `terminal.format`: plain text of the screen after N history lines; `server/tests/inspect.rs`, `ctl/tests/ctl.rs` |
| C19 | Scripting and config | 2 | `rusty_mux_script` | in-progress | JSON config of actions (chains of catalog requests, `${arg}`) and key bindings; `rusty-mux-ctl do`, `config check`, `config list`; `script/src/tests.rs`, `ctl/tests/ctl.rs` |
| C20 | Program status signalling | 2 | `rusty_mux_status` | in-progress | OSC 7501 (Program Status Protocol 0.4) reports, records per block, `BlockStatus` events with `Features::STATUS`, `terminal.status`; `status/src/*/tests.rs`, `server/tests/events.rs`, `server/tests/inspect.rs` |
| C21 | Tab indicators | 4 | `rusty_mux` clients | not-started | depends on C20 |
| C22 | Idle-pane scale (parking) | 5 | `rusty_mux_server` | not-started | spike 5 |
| C23 | Production controls (limits, audit) | 5 | `rusty_mux_server` | not-started | |
| C24 | Windows server | 4 | `rusty_mux_server`, `platform::pty` | not-started | spike 1 |
| C25 | Real-login remote, per-user accounts | 3 | `rusty_mux_auth` | not-started | |
| C26 | Personal / shared / login modes | 3 | `rusty_mux_server` | not-started | |
| C27 | Identity mapping (Tailscale to account) | 3 | `rusty_mux_auth` | not-started | |
| C28 | Disk-parked encrypted snapshots | 5 | `rusty_mux_server` | not-started | |

Update a row's status in the same PR that moves it.
