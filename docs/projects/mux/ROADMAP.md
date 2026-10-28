# Roadmap

Phases from the design brief §9. Each exit criterion is a test or a measured number. Phase N+1 starts only after the owner accepts phase N's go/no-go.

## Phase 0: engine prerequisites (done 2026-10-23, one target open)

Status: items 1, 2 and 4 landed (#872, #894, #939); item 3 landed as #930 plus #936. The MB/s floors (#889) are met on the 4 MB workloads (#1019; #889 closed 2026-10-25). Open: reliable ≤ 11.2 ms encode (#939). See CAPABILITIES.

1. #872 lands: instruction-count gate in `required-gate`.
2. #855 lands: per-code-point widths by default, mode 2027 opt-in; `vim_edit` matches the oracle.
3. Ring-buffer screen + recycled scrollback rows (backlog #1 + #2): `scroll_heavy` ≥ 16 MB/s, `ascii_throughput` ≥ 58 MB/s, memmove < 10% of instructions (callgrind); fidelity gate green; memory per 10k lines and peak RSS reported before and after.
4. Snapshot serializer (backlog #16): round-trip decode equals the source grid under the fidelity dump; 200x50 + 10k lines encodes in ≤ 11.2 ms and ≤ 7.8 MB.

Engine tuning stops here. Backlog #3 (GUI frame path) and #4 (printable fast path) wait unless a mux benchmark shows they are the bottleneck.

## Phase 1: vertical slice (Linux; Windows once spike 1 answers)

Runtime: `rusty_tokio` (DECISIONS), extended where needed; no tokio in any mux binary.

Scope: `core` (< 3k LOC, zero I/O, IDs via `rusty_uuid`), `proto` (`rusty_wire` + LEB128 varint; u32 BE length + u8 type + tagged fields; skip unknown ids; feature bits; < 4k LOC), `server` (PTYs via `platform::pty`, one `rusty_term_core` per PTY, snapshot then ready then raw byte tee, single-writer input, per-client viewport), and a throwaway TUI client over a Unix socket. No auth, TLS, roaming, parking, MCP or scripting.

Exit criteria:
- Two clients attach to one PTY and converge to the same grid (integration test).
- A client detaches and reattaches via snapshot; its grid equals the server's (integration test).
- An output flood does not starve input: a keystroke round-trips within a bound while the PTY floods (integration test; bound set in the PR).
- Fidelity gate and benchmark gate green.

## Phase 2: API and automation (done 2026-10-27)

Schema-first API, CLI, resumable events, block methods, program status, MCP. Exit: CLI and MCP are generated from one schema catalog (test); CLI exit codes 0/1/2/3/4/130 covered by tests; events resume after a disconnect without loss or duplication (test); status OSC round-trips server to client (test).

Started 2026-10-25 (owner go-ahead). One PR per step, in order:
1. Request schema catalog in `rusty_mux_proto::schema` (C17). Done (#1051).
2. Resumable event stream: sequence numbers and replay (C05). Done (#1057).
3. `rusty_mux_ctl`, generated from the catalog, with `--json` and exit codes (C04). Done (#1060).
4. Block methods and screen capture (C06, C18). Done (#1072): read-only `list` and `capture`, negotiated with `Features::INSPECT`.
4a. Rex parity, CLI: exit codes 3 (target) and 4 (unreachable), failure codes on the wire, targets by name, position, id suffix or path, ids from the block's environment (C04). Done (#1089).
4b. Rex parity, API: per-block-type methods `terminal.format`, `terminal.pwd` and `terminal.title` through one `BlockCall` (`Features::BLOCK_CALL`), replacing `capture`; `api list/describe` (C06, C17, C18). Done (#1091).
5. Program status OSC, `rusty_mux_status` (C20): OSC 7501 reports kept per block, a `BlockStatus` event when a state changes, and `terminal.status`. Done (#1092).
6. `rusty_mux_mcp` on `rusty_mcp_server`, generated from the same catalog (C07): `rusty-mux-mcp` over stdio, one tool per method that works on a connection of its own. Done (#1100).
7. Actions and key bindings from a JSON config, `rusty_mux_script` (C19): named chains of catalog requests with `${arg}` placeholders, run by `rusty-mux-ctl do`, checked by `config check`; `on` rules that `rusty-mux-ctl watch` runs when an event happens, including new `pwd-changed` and `title-changed` events. No language: logic lives in programs driving ctl or MCP. Done (#1128, #1138).

## Phase 2b: agent parity (done 2026-10-28)

From the [Rex gap check](REX-GAP-CHECK.md) §5 (owner, 2026-10-27). Started 2026-10-27, when the owner accepted Phase 2. One PR per step, in order. All seven steps merged by 2026-10-28, and the owner accepted the exit evidence the same day. The manual Windows check ([WINDOWS-MANUAL-CHECK](WINDOWS-MANUAL-CHECK-2026-10-28.md), #1212) passed after #1214 fixed Ctrl-] in Windows Terminal; the two-account check (R10) was not run. Exit: each step's behaviour has a server or ctl test, on Linux and on CI's Windows leg where the step does not depend on ConPTY pass-through (R1).

1. Terminal events (C29): program exited with its exit code, bell, progress, notification, behind a new feature bit; reply to the OSC 7501 query. Done (#1174).
2. Wait primitives (C30): `rusty-mux-ctl wait` on a block closing or its program exiting, with a timeout, exiting with the program's status; `create-block` takes an optional command, arguments and working directory per request (proto and server), so run-and-wait starts a program in a new block and exits with its status; wait for an event matching fields, with a timeout. Done (#1178): `wait --block`, `wait --event KIND --FIELD VALUE`, `run --wait`, `create-block --command --arg --cwd` behind `Features::LAUNCH`; a timeout exits 124.
3. Process info (C31): a `terminal.process` block method (child, foreground process, ancestors) and a process-changed event, using existing platform crates or `rusty_libc`. Done (#1183): `terminal.process` and `process-changed` behind `Features::PROCESS`, from `/proc` and `tcgetpgrp` (`rusty_libc`); on Windows the program's pid only, and no event.
4. Input and capture (C32): named keys encoded over `input`; `terminal.format` gains trim, unwrap and a VT format. Done (#1186): `terminal.send-keys` and `terminal.format --trim --unwrap --vt` behind `Features::CAPTURE`.
5. Labels and clients (C33): rename methods for all four kinds, block and client labels, label-changed events, client list and force-disconnect. Done (#1189): `rename-session|window|block|client`, `*-renamed` events, clients in `list` and `disconnect-client` behind `Features::LABELS`.
6. Agent setup (C34): `rusty-mux-ctl agent setup|status|remove` for Claude Code first, plus a placement-rules doc. Our own skill text; merges into settings without clobbering them. Done (#1190): the owner approved the writes (gap check K4) on 2026-10-27; skill, `SessionStart` hook and `agent hook`, with [AGENT-PLACEMENT.md](AGENT-PLACEMENT.md).
7. Window layout (C12): a split tree per window in core, split, zoom and close-pane methods, layout events; the attach client renders it. Server side done: `split-block`, `zoom-block`, `unzoom-window`, `close-block` closing the pane, `layout-changed` events and layouts in `list`, behind `Features::LAYOUT`. Done: server side in #1196; the attach client (#1200) draws the layout, with Ctrl-] commands for focus, split, zoom and close.

## Phase 3: identity and remote

Identity, login server mode, roaming transport, sharing, several servers per client (C35). Tailscale identity uses first-party `rusty_tail`, after the owner approves moving its library crates to `crates/libs/` (ADR-0003). Exit: a remote session survives a client IP change (test, after spike 4); login mode maps peer credentials or Tailscale identity to an account (test); a shared session enforces input leases (test).

## Phase 4: clients and Windows

Web and native clients, ConPTY server and client. Exit: the Phase 1 acceptance suite passes on Windows; web and native clients pass the convergence test.

Windows, ahead of the phase (owner, 2026-10-26): the server, `rusty-mux-ctl`, `rusty-mux-mcp` and the attach client run on Windows (#1109, #1136, #1142, #1148). `server/tests/phase1.rs` passes on CI's Windows leg except `reattach_after_more_than_four_mib_of_real_pty_output`, which stays Linux-only because ConPTY re-renders output. Still Linux-only: the attach client's lifecycle tests, `rusty-mux-mcp`'s stdio tests, and the ctl interrupt, `terminal.pwd`/`title` and `watch` rules tests. Open: spike 1 (R1) and the two-account check (R10).

## Phase 5: scale and hardening

Parking, encryption at rest, audit, upgrade without losing sessions (C36: running servers keep their sessions while a new version installs; live re-exec only if measured worth it). Exit: idle-terminal memory measured with and without parking (spike 5 numbers, ours); parked snapshots are encrypted and round-trip to the same grid (test); an outbound-connection audit of the release finds no egress.
