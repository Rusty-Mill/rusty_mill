# Capabilities

The 28 Rex capabilities from the design brief §3, plus C29–C36 from the [Rex gap check](REX-GAP-CHECK.md) (owner, 2026-10-27), re-baselined against **our** work, not phux's scorecard. Status: `not-started` / `in-progress` / `done` / `blocked`. Phases are from [ROADMAP.md](ROADMAP.md).

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
| C04 | CLI automation (`--json`, stable exit codes) | 2 | `rusty_mux_ctl` | done | #1060, #1089; `rusty-mux-ctl`, `ctl/tests/ctl.rs`; exit codes as Rex (0/1/2/3 target/4 unreachable/130); targets by id, name, position, id suffix or path (`ctl/src/target.rs`); ids default from `$RUSTY_MUX_SESSION`/`WINDOW`/`BLOCK`. Waits and run-and-wait continue in C30 |
| C05 | Resumable event stream | 2 | `rusty_mux_proto`, `rusty_mux_server` | done | #1057; `Features::EVENT_SEQ`, `server/tests/events.rs` (`a_resumed_stream_has_no_gap_and_no_repeat`). More event kinds continue in C29 and C31 |
| C06 | Per-resource (block) methods | 2 | `rusty_mux_server` | done | methods of a block type (`terminal.format`, `terminal.pwd`, `terminal.title`) through one generic `BlockCall`, checked against the catalog; also `input`, `resize`, `close-block` by block id; discovery with `list`; `server/tests/inspect.rs`; #1072, #1091. More methods continue in C31 and C32 |
| C07 | Agent / MCP integration | 2 | `rusty_mux_mcp` on `rusty_mcp_server` | done | #1100; `rusty-mux-mcp` (stdio): one tool per catalog method, same schema as `api describe`, each call run as ctl runs it; `mcp/src/tests.rs`, `mcp/tests/stdio.rs`. Agent setup continues in C34 |
| C08 | Command palette + directory browse | 4 | `rusty_mux` clients | not-started | |
| C09 | Self-hosted, no account, no telemetry | 1 | all | not-started | |
| C10 | Native scrollback and selection | 1 | `rusty_term_core`, `rusty_mux_tui` | in-progress | engine rows above |
| C11 | Session model (session/window/block/client) | 1 | `rusty_mux_core` | in-progress | model, single-writer input lease and event queue in `crates/apps/rusty_mux/crates/core`; the server wires it to PTYs |
| C12 | Window layout: splits and zoom, server-owned, rendered by clients | 2b | `rusty_mux_core`, `rusty_mux_server` | not-started | was client-native; changed by the owner 2026-10-27 (DECISIONS). Gap check G3 |
| C13 | Mosh-like feel (roaming, predictive echo) | 3 | `rusty_mux_transport` | not-started | spike 4 |
| C14 | Live sharing | 3 | `rusty_mux_auth`, `rusty_mux_server` | not-started | |
| C15 | Web client | 4 | `rusty_mux` web client | not-started | |
| C16 | Client matrix (TUI, web, native) | 4 | `rusty_mux` clients | not-started | |
| C17 | Self-describing API (schema-first) | 2 | `rusty_mux_proto` | done | #1051, #1091; `schema::CATALOG`; `rusty-mux-ctl api list` and `api describe` (parameters as JSON Schema) |
| C18 | Screen capture | 2 | `rusty_mux_server` | done | #1072, #1091; `terminal.format`: plain text of the screen after N history lines; `server/tests/inspect.rs`, `ctl/tests/ctl.rs`. Trim, unwrap and VT format in C32 |
| C19 | Scripting and config | 2 | `rusty_mux_script` | done | JSON config of actions (chains of catalog requests, `${arg}`), key bindings and `on` event rules (`${field}`); `rusty-mux-ctl do`, `watch`, `config check`, `config list`; `PwdChanged`/`TitleChanged` events with `Features::TERMINAL_EVENTS`; `script/src/tests.rs`, `ctl/tests/ctl.rs`, `server/tests/events.rs`; #1128, #1138 |
| C20 | Program status signalling | 2 | `rusty_mux_status` | done | #1092; OSC 7501 (Program Status Protocol 0.4) reports, records per block, `BlockStatus` events with `Features::STATUS`, `terminal.status`; `status/src/*/tests.rs`, `server/tests/events.rs`, `server/tests/inspect.rs`. The `?` query reply came with C29 (#1174); Windows pass-through is R1 |
| C21 | Tab indicators | 4 | `rusty_mux` clients | not-started | depends on C20 |
| C22 | Idle-pane scale (parking) | 5 | `rusty_mux_server` | not-started | spike 5 |
| C23 | Production controls (limits, audit) | 5 | `rusty_mux_server` | not-started | |
| C24 | Windows server | 4 | `rusty_mux_server`, `platform::pty` | done | Server, ctl and attach client on Windows over AF_UNIX (#1109, #1136, #1142, #1148); `server/tests/windows.rs`, `server/tests/phase1.rs`, `ctl/tests/ctl.rs` on CI's Windows leg. Limits: RISKS R1, R10–R15 |
| C25 | Real-login remote, per-user accounts | 3 | `rusty_mux_auth` | not-started | |
| C26 | Personal / shared / login modes | 3 | `rusty_mux_server` | not-started | |
| C27 | Identity mapping (Tailscale to account) | 3 | `rusty_mux_auth` | not-started | |
| C28 | Disk-parked encrypted snapshots | 5 | `rusty_mux_server` | not-started | |
| C29 | Terminal events: program exited (with exit code), bell, progress, notification; OSC 7501 query reply | 2b | `rusty_mux_proto`, `rusty_mux_server` | done | #1174; `ProgramExited`, `Bell`, `ProgressChanged`, `Notification` (kinds 16–19) with `Features::PROGRAM_EVENTS`; the `?` query answered in reply order by `rusty_term_core`; `server/tests/events.rs`, `server/tests/windows.rs` (exit code), `ctl/tests/ctl.rs`. Bell, progress and notification on Windows need ConPTY pass-through (R1). Gap check G8, G18 |
| C30 | Wait primitives: wait for a block to close or its program to exit, run a command in a new block and wait, wait for an event | 2b | `rusty_mux_proto`, `rusty_mux_server`, `rusty_mux_ctl` | done | #1178; gap check G13; builds on C29's `program-exited`. `create-block` takes `command`, `arg` and `cwd` (`Launch`, `Features::LAUNCH` bit 11); `rusty-mux-ctl wait --block`, `wait --event`, `run [--wait]` (`ctl/src/wait.rs`), exit with the program's status or 124 on timeout; `ctl/tests/ctl.rs`, `server/tests/events.rs` |
| C31 | Running-process info per terminal, and a process-changed event | 2b | `rusty_mux_server` | done | #1183; gap check G10. `terminal.process` lists `PID<TAB>NAME<TAB>COMMAND` per line, the foreground process first, then its ancestors, the block's program last (`server/src/process.rs`, from `/proc`); `ProcessChanged` (kind 20, `pid` and `name` of the new foreground group) with `Features::PROCESS` bit 12, checked on output and every 500 ms; `server/tests/inspect.rs`, `server/tests/events.rs`, `ctl/tests/ctl.rs`. Windows lists only the program's pid and sends no event (no foreground group under ConPTY) |
| C32 | Named-key input; capture trim, unwrap and VT format | 2b | `rusty_mux_proto`, `rusty_mux_ctl`, `rusty_mux_server` | done | #1186; gap check G11, G12. `terminal.send-keys` (repeated `key`: a character or a named key after `ctrl+`, `alt+`, `shift+`) encoded by the server as xterm does, cursor keys following the program's DECCKM, under the lease (`server/src/keys.rs`); `terminal.format` takes flags `trim`, `unwrap` and `vt` (SGR per style change) behind `Features::CAPTURE` bit 13, a new `Flag` parameter type (`--trim` alone in ctl, a boolean in MCP and actions); `server/src/terminal.rs`, `server/tests/inspect.rs`, `ctl/tests/ctl.rs`. HTML capture not built |
| C33 | Labels and renames on all four kinds; client list and force-disconnect | 2b | `rusty_mux_core`, `rusty_mux_server` | done | #1189; gap check G4, G20. Every kind has a name (blocks and clients start unnamed); `rename-session`, `rename-window`, `rename-block`, `rename-client` (one `Rename` message, `Mux::rename`); `session-renamed`, `window-renamed`, `block-renamed`, `client-renamed` (kinds 21–24); `list` adds a client entry per connection (name, attached block); `disconnect-client` ends another client's connection, freeing its lease; all behind `Features::LABELS` bit 14; names are at most 256 bytes. ctl targets name blocks and clients too; `core/src/mux/tests.rs`, `server/tests/labels.rs`, `ctl/tests/ctl.rs`. No client may disconnect itself; anyone on the socket may disconnect anyone (see DECISIONS) |
| C34 | Agent setup: install, check and remove a skill and session-start hook for known agents | 2b | `rusty_mux_ctl` | done | this PR; gap check G17, K4 (writes approved by the owner 2026-10-27). `rusty-mux-ctl agent setup\|status\|remove [--dir DIR]` for Claude Code (`$CLAUDE_CONFIG_DIR` or `~/.claude`): our own skill at `skills/rusty-mux/SKILL.md` and one `SessionStart` hook running `rusty-mux-ctl agent hook`, merged into `settings.json`; `remove` restores the file byte for byte when it is otherwise unchanged (`ctl/src/agent.rs`, `ctl/src/agent/tests.rs`, `ctl/tests/agent.rs`). Placement rules: [AGENT-PLACEMENT.md](AGENT-PLACEMENT.md). Other agents not started |
| C35 | Several servers: saved hosts, a server per call | 3 | `rusty_mux_ctl` | not-started | gap check G23 |
| C36 | Upgrade without losing sessions | 5 | `rusty_mux_server` | not-started | gap check G24. Rex level: a running server keeps its old version and sessions until it stops, and new connections get the new version. Beyond Rex: live re-exec with PTY adoption, which would also keep a shared server's sessions |

Update a row's status in the same PR that moves it.
