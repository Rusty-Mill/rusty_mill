# Decisions

| Date | Decision | Why | Owner/CC |
|---|---|---|---|
| 2026-10-21 | Build fresh on rusty_mill crates (brief option 3), borrowing phux's protocol ideas | phux's Windows gap is structural; its snapshot bootstrap needs a forked libghostty; most components already exist here (`MUX-ADOPT-DECISIONS.md` §1) | Owner |
| 2026-10-21 | ~~tokio is the async runtime~~ | Superseded by the 2026-10-22 `rusty_tokio` row | Owner |
| 2026-10-21 | `rusty_term` is the single VT engine; libghostty-vt is a comparison oracle only | Reuse rule; no Zig/C toolchain; sovereignty | Owner |
| 2026-10-21 | DECCOLM never resizes; honor mode ?40, default off. Verify against Ghostty first | Matches Alacritty and kitty; resizing is xterm opt-in (#856) | Owner |
| 2026-10-21 | #850 (tokio-free `l13`) is low priority | The mux does not need `l13` in Phase 1 | Owner |
| 2026-10-22 | Project and crate prefix: `rusty_mux` | Owner confirmed in the project thread | Owner |
| 2026-10-22 | Crate names `rusty_mux_*` (core, proto, server, tui, ctl, mcp, transport, auth, status, script) as listed in CAPABILITIES | Owner confirmed in the project chat; each crate is still created only when its phase starts | Owner |
| 2026-10-22 | `rusty_tokio` is the async runtime for every `rusty_mux_*` crate; extend `rusty_tokio` where the mux needs more | Sovereignty; first-party runtime. AF_UNIX works on Windows (`rusty_tokio/tests/unix_windows.rs`); `peer_cred` is Linux and macOS only (`rusty_tokio/src/io/unix.rs:815-819`), see RISKS R10. No tokio in any mux binary | Owner |
| 2026-10-23 | Accept #930 (ring buffer) without the MB/s floors; #889 stays open until they are met | memmove target met, instructions down 50-74%, memory flat; the remaining gap is memory layout | Owner |
| 2026-10-25 | Start Phase 2; CLI and MCP are generated from one request catalog in `rusty_mux_proto::schema` | Phase 1 complete (#1030); one catalog keeps the two front ends from drifting | Owner |
| 2026-10-25 | Match Rex's public CLI and API meanings: exit codes 3 (target missing or ambiguous) and 4 (server unreachable); targets by id, name, position or id suffix; per-block-type methods (terminal format, pwd, title); `api list/describe`. #1072 merges first, then parity PRs before step 5 | Rex docs (automate/cli, automate/server-api) read after #1072; compatibility stays a non-goal, meanings follow Rex | Owner |
| 2026-10-26 | Program status uses OSC 7501, the public Program Status Protocol (draft 0.4), rather than an OSC of our own | Rex's programs and others already emit it; the brief allows interoperating; one parser serves both. Not yet supported: the `?` query | Owner (2026-10-26: fine to use if it fits the need) |
| 2026-10-26 | Scripting (C19) is declarative: a JSON config of named actions (chains of catalog requests with `${arg}` substitution) and key bindings, run by `rusty-mux-ctl do`; no embedded language and no new dependency | Rex's actions are mostly request chains; real logic belongs in external programs driving ctl or MCP; a language is a large, hard-to-reverse surface | Owner (2026-10-26: "Declarative actions, no language") |
