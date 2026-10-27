# Rex gap check, 2026-10-27

What Rex does, from its public docs read on 2026-10-27, against what rusty_mux has on `main` (c4a0d031). Input: the owner's Rex capability brief (2026-10-27, not in the repo). Everything here is paraphrased; no Rex text, schema or code is copied. Nothing here changes [ROADMAP.md](ROADMAP.md) or [CAPABILITIES.md](CAPABILITIES.md): §5 lists proposals that wait for the owner.

Tags: **[docs]** a Superlogical docs page under `superlogical.com/rex/docs/` (named in brackets), **[demo]** a Superlogical or Hashimoto post, **[unverified]** inferred or single-source. rusty_mux evidence cites files on `main`.

## 1. Corrections to the brief

| Brief said | Docs say |
|---|---|
| No Windows build | Rex servers already run on Windows; Windows, Linux and iOS clients are in progress, outside testing [docs: home]. Windows is a lead for rusty_mux, not a lasting differentiator |
| No public changelog | Release notes per version are readable through the beta Community MCP server, behind a Discord login [docs: beta/community-mcp] |
| Input simulation covers keyboard and mouse | Documented: typed text (`send`), named keys (`send-key`), and a terminal `write` block method. No mouse input is documented [docs: automate/shell-scripting, reference/lua]. Mouse stays [demo] |
| SSH-style remote logins | Not in the docs. Remote is HTTP(S) listeners: built-in Tailscale, Tailscale Serve, a tailnet address, or plain HTTP behind an explicit opt-in [docs: linux/remote-access] |
| OSC 7501 "spike open" in rusty_mux | Implemented (#1092, C20). Spike 1 is the general ConPTY pass-through question (R1) |
| Gap rows "Unknown" | Most are now present in rusty_mux; see §3 |

## 2. Object model, resolved

Four kinds, each with a permanent, never-reused id (typed prefix plus a UUID) and an optional, changeable label [docs: automate/cli, automate/server-api]:

- **Session**: a group of related work; the server keeps it alive with no client attached.
- **Window**: a tab in a session. It **holds a layout of blocks**.
- **Block**: one application in a window; today only terminals.
- **Client**: a connected program, such as the app; targeted when asking the app itself to act.

Panes are not a fifth kind: a pane is a block's place in its window's layout, and a split creates a block beside another. The layout lives on the server: `session.new_split`, a `session_view_changed` event with a revision, `zoom`, and a Lua layout builder of nested horizontal and vertical splits [docs: automate/server-api, reference/events, reference/lua]. Lua session methods also mention "layers" [docs: reference/lua]; what a layer is (floating or stacked blocks?) is [unverified].

rusty_mux has the same four kinds (`crates/core/src/entity.rs`, `crates/core/src/event.rs`) but no layout: a window is a list of blocks. See conflict K1.

## 3. Gap table

Status: **present**, **partial**, **absent**, **ahead** (rusty_mux does more), **by decision** (an owner decision chose another shape).

| # | Rex capability | Source | rusty_mux | Status | Evidence / what is missing |
|---|---|---|---|---|---|
| G1 | Server-owned persistent sessions | docs | C01 | partial | Blocks outlive clients; server is not a daemon. Rex's personal server starts on first use and exits ~10 s after the last session and connection [docs: linux/server-modes] |
| G2 | Four object kinds | docs | C11 | present | `crates/core/src/entity.rs` |
| G3 | Window layout: splits, zoom, layout events | docs | C12 | absent | C12 plans client-native splits; see K1 |
| G4 | Ids plus changeable labels on all four kinds | docs | C11 | partial | Sessions and windows take a name at creation only; no block or client labels, no rename methods, no label-changed events |
| G5 | Targets by id, label, position, id suffix; exit codes 0/1/2/3/4/130 | docs | C04 | present | #1089, `crates/ctl/src/target.rs` |
| G6 | Self-describing API (list, describe, call) | docs | C17 | present | `api list`, `api describe`; every catalog method is a ctl command, which covers `call` (`crates/ctl/src/cli.rs`) |
| G7 | Event stream for clients and scripts | docs | C05 | ahead | Resumable with sequence numbers (#1057). Rex documents no resume or replay [docs: reference/events, automate/lua-events] |
| G8 | Event coverage | docs | C05, C19 | partial | Have: session, window, block, client, lease, status, pwd, title. Missing: child exited with exit code, bell, foreground process changed, desktop notification, progress report, clipboard written, status removed, label changes, active window changed, layout changed, config reloaded, client connection state |
| G9 | Per-block-type methods | docs | C06 | present | `terminal.format`, `pwd`, `title`, `status` (`crates/proto/src/schema.rs`) |
| G10 | Running-process info (child and foreground, as JSON) | docs | none | absent | Rex: terminal `process` method and `terminal.process_changed` event (foreground, ancestors, generation) [docs: reference/lua, reference/events] |
| G11 | Screen capture with options | docs | C18 | partial | Plain text after N history lines. Missing: styled (HTML) and raw VT formats, trim, unwrap. Rex keeps ~64 MiB of scrollback; ours is 10,000 rows ([REPLAY.md](REPLAY.md)) |
| G12 | Input: text, named keys | docs | `input` | partial | Raw bytes only; no named-key encoder (`send-key` equivalent). Mouse: neither documents it |
| G13 | Wait primitives | docs | `watch` | partial | No wait for a block to close or its program to exit, no run-and-wait returning the program's exit status, no wait-for-event with a filter and timeout. `watch` runs rules but does not block a caller |
| G14 | Scripting language, timers, exec | docs | C19 | by decision | Declarative JSON actions and `on` rules, no language (DECISIONS 2026-10-26). Rex adds timers and process exec; ours leave those to external programs. Timers were noted as not approved |
| G15 | Actions, bindings, key modes and sequences | docs | C19 | partial | Actions and binds present; no key modes or multi-key sequences; palette metadata (title, keywords) absent |
| G16 | Context env vars in child processes | docs | C04 | present | `RUSTY_MUX_SESSION`, `WINDOW`, `BLOCK`, `SOCKET` (`crates/server/src/state.rs`). Rex also sets a default-client var and puts its CLI first on `PATH` |
| G17 | Agent integration | docs | C07 | partial | MCP server present (Rex has none for control). Absent: an installer that adds a skill and a session-start hook to known agents, with status and remove; documented placement rules ("here", "focused", label wins) |
| G18 | Program Status Protocol, OSC 7501 0.4 | docs | C20 | present | #1092. Missing: the `?` query reply; Windows pass-through unverified (R1) |
| G19 | Status indicators in UI, priority inbox, push | docs | C21 | absent | Client-side, Phase 4 |
| G20 | Client list, labels, force-disconnect, ask a client to act | docs | none | absent | `list` covers sessions, windows and blocks only |
| G21 | Remote listeners with caller identity (Tailscale) | docs | C25, C27 | absent | Phase 3. First-party `rusty_tail` exists (`crates/apps/rusty_tailscale`, userspace tailnet on `rusty_tokio`); see K3 |
| G22 | Server modes: personal, shared, login | docs | C26 | absent | Phase 3 |
| G23 | Several servers: saved hosts, per-call server target | docs | none | absent | Not in CAPABILITIES |
| G24 | Upgrade without losing sessions | docs | none | absent | Rex's login server keeps per-account servers running across upgrades. phux-style re-exec with PTY adoption was noted in the design brief §4 |
| G25 | Reattach from another device | docs | C03 | partial | Reattach by snapshot works locally; remote needs G21 |
| G26 | Multiplayer, web, iOS | demo, docs | C14, C15, C16 | absent | Rex documents iOS only (in progress); sharing and web are [demo] |
| G27 | Windows server | docs | C24 | ahead (for now) | Public on our side; Rex's is internal |
| G28 | No telemetry, no hidden egress | docs | C09 | ahead | Rex for Mac's update check cannot be turned off [docs: reference/privacy]; rusty_mux makes no outbound calls by design (CHARTER) |
| G29 | Command palette | docs | C08 | absent | Client-side, Phase 4 |

## 4. Conflicts and flags

- **K1, server-side layout vs the design brief.** The design brief §2 says Rex splits are client-native with no server pane tree, and C12 follows it. The docs now show layout on the server (split, zoom, layout revisions, layout builder). Matching Rex's capability for agents (split beside me, zoom on failure, read the layout) needs a window layout in `rusty_mux_core` and events for it. This reverses a design assumption: owner call.
- **K2, scripting.** Rex's Lua has timers, process exec and blocking waits. DECISIONS keeps us declarative with no language and no new dependency. G13 waits can live in ctl without a language; timers and exec stay out unless the owner reopens C19.
- **K3, Tailscale.** `rusty_tail` is first-party, so no new third-party dependency, but it sits in `crates/apps/`. ADR-0003 forbids app-to-app edges, so using it from `rusty_mux_auth` means moving its library crates to `crates/libs/` first: an owner gate (moving crates).
- **K4, agent installer.** A skill-and-hook installer writes into other tools' settings files (`~/.claude/settings.json` and the like). That is user config outside the repo; it needs the owner's go-ahead and must merge without clobbering. Write our own skill text; do not reuse Rex's guide.
- **K5, clean room.** OSC 7501 is a published open protocol (already decided). Event and method meanings may follow Rex (DECISIONS 2026-10-25); names, schemas and docs prose must be our own. Nothing in §5 needs Rex code.
- **K6, rusty_tokio and deps.** None of §5 needs tokio or a third-party crate. Process info (G10) needs per-OS process queries: `/proc` on Linux, Toolhelp on Windows, `libproc` on macOS through existing platform crates or `rusty_libc`; check before adding anything.
- **K7, scrollback.** Rex's 64 MiB capture budget is ~6x ours. Raising it touches the snapshot and reattach budgets in REPLAY; measure first.

## 5. Proposed roadmap additions (not applied)

For Nano's use case (agents and long-running work), in this order. Each would be one PR, gated by the owner like Phase 2's steps.

1. **Terminal events (G8).** `child-exited` with exit code, `bell`, `progress` (OSC 9;4), `notification` (OSC 9/777), behind a new feature bit. Small; the owner listed bell and exit as not yet approved. Unlocks G13.
2. **Wait primitives (G13).** `rusty-mux-ctl wait` (block closed or program exited, with a timeout; exits with the program's status) and `run --wait`; an event wait with a field filter and timeout. Agents need this most.
3. **Process info (G10).** `terminal.process` block method (child pid, foreground process, its ancestors) and a `process-changed` event.
4. **Named-key input and capture options (G12, G11).** A key-name encoder over `input`; `terminal.format` gains trim, unwrap and a VT format.
5. **Labels everywhere (G4, G20).** Rename methods for all four kinds, block and client labels, label-changed events, client list and force-disconnect.
6. **Agent installer (G17).** `rusty-mux-ctl agent setup|status|remove` for Claude Code first, plus a placement-rules doc. After K4.
7. **Window layout (G3).** Only if the owner reverses C12 (K1): splits, zoom, layout events in core.
8. **Remote and multi-device (G21–G25).** Phase 3 as planned, plus saved hosts and upgrade survival. After K3 and spike 4.
9. **OSC 7501 query (G18).** Reply to `?`. Tiny; can ride with item 1.

Not proposed: a scripting language, timers or exec (K2); mouse input (undocumented on both sides).

## 6. Sources

Rex docs pages read 2026-10-27: home, automate/cli, automate/server-api, automate/agent-skill, automate/lua-scripts, automate/lua-events, automate/shell-scripting, customize/actions, build/program-status, reference/events, reference/lua, reference/privacy, linux/server-modes, linux/remote-access, beta/community-mcp. Not read: customize/config, keyboard-shortcuts, key-sequences, key-modes, linux/installation, shared-server, login-server, upgrading.
