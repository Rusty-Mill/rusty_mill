# Decisions — rusty_term (with rusty_mux)
Append-only. Newest last. Never edit a row; supersede it with a new row (`supersedes D<n>`). Only `rusty-term-expert` appends.
Tags: `[stated]` = Nano's own words, quoted in Source (ask before changing). `[default]` = settled by an agent, Dottius, a review or an unquoted paraphrase (revise with a new row saying why). `[open]` = no source found.
Source markers: **primary** = Nano's message or PR comment itself. **†** = Nano's words quoted by an agent in a repo doc or remind-me memory, with date and place but no link to the message; treat as `[stated]`, and if Nano disputes one, record a superseding row.
The detailed logs stay where they are: mux `docs/projects/mux/DECISIONS.md`, `docs/projects/term/NOTES.md` §1, `docs/projects/term/conhost/design.md` (D1–D6). Their "Owner" column is agent-written; a row there is `[default]` here unless a row below quotes Nano.

## Pack and routing

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D1 | Experts run in cloud sessions with no local setup | Nano works cloud-only across several machines | a local-only expert (as the Pythrust expert became) | [stated] | primary: Nano, harvest prompt, Rusty Term thread 2026-11-03 19:31Z: "Experts must run in cloud sessions with no local setup." |
| D2 | If `subagent_type: "rusty-term-expert"` is unavailable, spawn a general-purpose Agent whose prompt is the agent file's full body, read fresh, plus the question; that counts as the expert; no copies of the file's text | cloud threads failed to load repo agents | improvising an answer without the expert | [stated] | primary: same prompt: "if subagent_type "<slug>-expert" isn't available, spawn a general-purpose Agent whose prompt is the full body of the agent file, read fresh at call time, plus the question. That counts as the expert; improvising an answer without it is still forbidden. The agent file stays the one source of truth: no copies of its text." |
| D3 | Three-tier escalation (answer, default, escalate); the expert is the single writer | each answer recorded once | callers writing decisions | [stated] | primary: same prompt: "Escalation stays three-tier, and the expert stays the single writer." |
| D4 | Harvest provenance: [stated] only with a quoted source in Nano's words; anything settled by Dottius, a thread or a review is [default]; untraceable is [open]; never promote | a pack is only as trustworthy as its tags | promoting on silence | [stated] | primary: same prompt: "[stated] only with a quoted source in my words … Anything settled by Dottius, a thread or a review is [default]. Untraceable is [open]. Never promote." |
| D5 | Precedence: code and repo docs beat memory; newer beats older; conflicts go to OPEN_QUESTIONS with both sources | one rule for every overlap | silent picks | [stated] | primary: same prompt: "code and repo docs beat memory; a newer decision beats an older one; a conflict goes to OPEN_QUESTIONS with both sources." |
| D6 | Slug `rusty_term`; mux merged into it 2026-11-03; Mill Board projects `rusty_term` and `rusty_mux` stay separate | one expert for the merged project | a separate mux expert | [stated] | primary: same prompt: "Slug rusty_term (mux merged into it 2026-11-03; Mill Board projects stay separate). Expert: rusty-term-expert. Pack: docs/projects/rusty_term/handoff/." |
| D7 | GUI design questions route to `rusty-term-design-expert` and its pack `docs/projects/rusty_term/gui-design/`; it needs the Paper MCP server running locally on Windows, so in the cloud it answers from its pack only and can't edit designs | design has its own pack and tool | folding GUI design into this pack | [stated] | primary: same prompt: "GUI: route to the existing rusty-term-design-expert and its pack in docs/projects/rusty_term/gui-design/. Note in the routing table that it depends on the Paper MCP server running locally on Windows…". The agent and pack are not on main: OPEN_QUESTIONS Q4 |
| D8 | In-flight work (#1435, C71) is status, not decisions | it belonged to the old mux thread | — | [stated] | primary: same prompt: "In-flight work (#1435, C71) stays with the old mux thread; list it as status, not decisions." Both finished by harvest time (SPEC § Status) |
| D9 | No new dependencies for the pack; keep it terse | — | — | [default] | Nano tagged it [default] in the harvest prompt |
| D10 | Until `.claude/agents/rusty-term-design-expert.md` exists, a GUI **design** question (look, layout, taste) escalates to Nano; GUI **code behaviour** questions are answered here from code | D7's target is missing; taste is never a default | answering design from remind-me memories alone | [default] | harvest 2026-11-03; Q4 |

## Terminal core (`rusty_term_core`)

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D11 | `rusty_term_core` is the only VT engine (term and mux); libghostty is a comparison oracle only, never on a runtime path | sovereignty, no Zig/C toolchain | libghostty-vt | [default] | mux DECISIONS 2026-10-21; NOTES D1. Reviewer relay on #888 (comment 6345218936) reports Nano's "Yes and approved", not Nano's own comment |
| D12 | DECCOLM never resizes grid, window or PTY; mode ?40 honoured, default off; the three vttest DECCOLM diffs vs libghostty are deliberate | the mux server owns PTY size | xterm/Ghostty resize | [default] | NOTES D5, D6; FIDELITY.md; #888 relay as D11 |
| D13 | Widths per code point by default; regional indicators 2; grapheme clustering only with DEC mode 2027; no config key | per-code-point apps misplaced the cursor | clustering by default | [default] | NOTES D7; #894. Config key: NOTES §3 Q7, unconfirmed |
| D14 | BCE: erases fill with the current SGR background | terminfo advertises `bce` | — | [default] | NOTES D8; #862 |
| D15 | CI perf gate counts instructions (cachegrind); MB/s never fails a build; a PR that lowers a count lowers its ceiling to about measured +10% | deterministic on shared runners | wall-clock gates | [default] | NOTES D4; HANDOFF §5; `bench/ir_ceilings.txt` |
| D16 | Throughput floors are judged on the 4 MB fresh-grid workloads; they pass (#1019); #889 closed 2026-10-25 | newer evidence | judging the 2026-10-23 per-iteration numbers | [default] | `docs/projects/mux/ROADMAP.md` Phase 0; card 80 comment. Supersedes HANDOFF §3.1 "floors not met" (conflict: Q2) |
| D17 | 8-byte cells over an interned style table; RTCP/RTSN bytes unchanged by the change; worst-case styling cost accepted | 24.4 → 8 B/cell | contiguous scrollback (row 6, deferred) | [default] | NOTES §2 Grid API ("accepted by the owner (Q1 and G6, 2026-10-25)", no quote); #1079, #1090 |
| D18 | Snapshot decoder input is untrusted: MAX_DIM 4096, MAX_CELLS 2^25, MAX_LINES 2^20, validate before allocate, peak-allocation test | attach reads peer bytes | — | [default] | HANDOFF §6; `core/src/grid/snapshot.rs`; `tests/snapshot_alloc.rs` |
| D19 | Incremental attach (per-row seqno, stable row numbers) stays on paper until the mux protocol work that consumes it; agree its shape with the mux side first | change the wire once | shipping it ahead of a consumer | [default] | `docs/projects/mux/DELTA.md`; card 83 |
| D20 | Public API `Grid::sync_output_until` in `rusty_term_core` | synchronized-output timeout wake | — | [stated] | primary: Nano, PR #1112 comment 6422673193: "I approve adding the public API: Grid::sync_output_until into rusty_term_core." |
| D21 | Scroll step 2 (O(1) rotate) dropped; step 3 (lazy blanking) parked until scroll speed is a real need | ≤6% gain for 32 call sites; floors pass | doing either now | [default] | cards 79, 80 (agent comments); mem_0a11517ab1814e3680194953be88213f (paraphrase, no quote) |
| D22 | Mux crates link `rusty_term_core` (without `l13`), never the `rusty_term` front-end; today only `rusty_mux_server` and `rusty_mux_tui` link it | the front-end is the windowed/TUI app | linking `rusty_term` | [default] | HANDOFF §4; `crates/apps/rusty_mux/crates/{server,tui}/Cargo.toml` |
| D23 | No new third-party dependency, lockfile, dependency or toolchain bump without Nano's approval; fix advisories, never suppress | sovereignty | suppressing RUSTSEC findings | [default] | HANDOFF §4; NOTES D15 (agent-reported "a", "Close 944") |
| D24 | `rusty_term` runs on `rusty_tokio`; no tokio, no direct `parking_lot`; fair selection via `runtime/fair.rs::fair_select` | sovereignty; starvation fix on #955 | tokio | [default] | #955, #956; NOTES §2 |

## Mux server and protocols

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D25 | Build fresh on rusty_mill crates; capability parity with Rex, not protocol compatibility; crate prefix `rusty_mux_*` | Windows gap in phux; most parts exist | adopting phux; tmux/Rex compatibility | [default] | mux DECISIONS 2026-10-21, 2026-10-22; CHARTER.md; #888 relay |
| D26 | `rusty_tokio` is the runtime for every `rusty_mux_*` crate; no tokio in any mux binary | sovereignty | tokio (superseded row 2026-10-21) | [default] | mux DECISIONS 2026-10-22; #888 relay ("Yes and approved") |
| D27 | Capabilities are negotiated by `Features` bits; take the next free bit/type/kind/field from `proto/src/message.rs` on main | old clients keep working | a protocol version number; numbers from docs | [default] | `proto/src/message.rs`. Supersedes the mux handoff's "feature bit 32 next" (conflict: Q8) |
| D28 | Window layout is server-owned: a split tree of blocks per window, zoom and layout events; clients render it | agents edit layout through the API | client-native splits | [stated] | † mux DECISIONS 2026-10-27: Owner (2026-10-27: "yes") |
| D29 | Scripting (C19) is declarative named actions plus key bindings run by ctl; no embedded language, no new dependency | — | an embedded language | [stated] | † mux DECISIONS 2026-10-26: "Declarative actions, no language" |
| D30 | Event hooks as declarative `on` rules (`rusty-mux-ctl watch`) | — | — | [stated] | † mux DECISIONS 2026-10-26: "Add it" |
| D31 | Program status uses OSC 7501 (Program Status Protocol draft 0.4) | public protocol | an OSC of our own | [stated] | † mux DECISIONS 2026-10-26: "fine to use if it fits the need" |
| D32 | Windows R10: owner-only DACL on the socket plus a peer token-SID check, failing closed | mirrors Linux 0600 + SO_PEERCRED | one layer | [stated] | † mux DECISIONS 2026-10-26: "Accept both layers"; mem_e1e737d806b64d55b6502c355029afbe |
| D33 | Phase 2b accepted; not run on Windows: R10 two-account check, the other layout keys, reattach after a clean detach | — | — | [stated] | primary: Nano, PR #1218 comment 6482365751: "I accept Phase 2b. Not run on Windows: R10 (two-account check), the other layout keys (-, arrows, z, x), and reattach after a clean detach." |
| D34 | A client's principal rides only client-connected and client-connection-changed events (plus client entries and whoami) | a client's principal never changes | every client event | [stated] | † mux DECISIONS 2026-11-01: "Confirmed: principal only on client-connected and client-connection-changed" |
| D35 | C33: any client may rename anything and disconnect any client but itself; acceptable for local use only, to be restricted before shared mode (card 507) | — | — | [default] | mux DECISIONS 2026-10-27 ("confirmed by the owner 2026-10-28 for local use", no quote) |
| D36 | Adopt the Rex app gaps as C45–C72; rows resting only on bundle names are *(inferred)* directions, not claims about Rex; each row needs Nano's own go | — | — | [stated] | † mux DECISIONS 2026-11-02: "Add all of the gaps". Inferred/own-go rule: REX-APP.md, [default] |
| D37 | Every capability row gets a DECISIONS row, *Proposed* until Nano accepts it on the PR; Dottius accepts owner decisions only when Nano states them on the PR | Dottius's review rule | paraphrased acceptance | [default] | mux handoff 2026-11-03 §2; PR #1307 comment 6550015980 |
| D38 | C71 `terminal.shell`: program plus idle/busy from the process table always, plus OSC 133 prompt phase and last exit code when the shell emits them (`ShellPhase` ungated in `rusty_term_core`) | works with any shell; richer where integrated | process table only (recommended) | [default] | mem_42b8917ffac24e429919417c5d1456de (decision-card pick, no quote); mux DECISIONS 2026-11-03; #1441 |
| D39 | C26 login mode ships without PAM; PAM comes later and needs approval to link libpam | native dependency gate | PAM now | [default] | mem_9cbb3255b7a141a9b4184de4e6c52743 (card pick, no quote); mux DECISIONS 2026-11-01 |
| D62 | Test fixtures in `rusty_mux_server` `process.rs`: a canned `/proc/<pid>/stat` line (e.g. a `comm` holding a space and `)`) is an inline byte-string literal in the test that uses it; promote it to a `const` in `mod tests` only once a second test reuses it | matches the module: `proc_files_are_parsed` binds `let stat = b"…"` inline, and `mod tests` keeps `const` for shared values (`CHILD`); keeps the input next to its assertions | a module-level `const` for a single-use fixture | [default] | `crates/apps/rusty_mux/crates/server/src/process.rs:193,379-381`; asked 2026-11-03 (smoke test) |

## Transport

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D40 | Phase 3 transport order from spike 4, one PR per step; QUIC ruled out | — | QUIC | [stated] | † mux DECISIONS 2026-10-28: chose "Approve the order" on the decision card; mem_95bc7a79df7c49debb80d0ada321bfb8 |
| D41 | Remote auth: SSH bootstrap first (as mosh), Tailscale identity probably next; TLS + token later (card 506) | smallest option, no new gate | TCP + `rusty_tls` listener first | [stated] | † mux DECISIONS 2026-10-29: "We start with A with plans to incorporate the others. D would probably be next" |
| D42 | The client reaches the remote socket both ways, SSH socket forwarding first | — | — | [stated] | primary: Nano, PR #1236 comment 6502493327: "I am confirming the “Both, B first” transport choice." |
| D43 | Predictive echo: the listed picks (fixed 50 ms grace, off when echo mode is unknown, …); masked-field bound is option B, one round trip of typing | — | — | [stated] | primary: Nano, PR #1245 comments 6508034930 "agree with your picks" and 6510217696 "I choose B" |
| D44 | Reconnect keeps a TAKEOVER client and its lease for 30 s (`ORPHAN_GRACE`); the last confirmed token is accepted until the new one is used | a lost Welcome must not drop the client | — | [stated] | primary: Nano, PR #1251 comment 6523175493: "Confirmed: 30 s grace including the lease, and accept the last confirmed token until the new one is used" |
| D45 | UDP transport: the four proposed answers in UDP-TRANSPORT.md (24 h association idle, lease released after 30 s, UDP only with `--udp`, …) | — | — | [stated] | † mux DECISIONS 2026-10-30: "I agree with all four" |
| D46 | After an SSH takeover the client re-asks for a lease UDP silence released; the server never hands it back by itself | never steal a lease taken meanwhile | server-side return | [stated] | primary: Nano, PR #1293 comment 6547150752: "Confirmed: the client re-asks for the lease after a takeover" |
| D47 | UDP keepalive 3 s, path lost after 6 s; the client sends first to an unheard address, with backoff | — | RFC 8085's 15 s | [stated] | † mux DECISIONS 2026-10-31: "Leave it as is" |
| D48 | C13 stays in progress until its four unbuilt pieces land | — | closing it at #1283 | [stated] | † mux DECISIONS 2026-10-31: "Keep C13 open until it all lands" |
| D49 | `rusty_mux_transport` is sans-I/O: no clock, socket or thread; datagrams sealed with ChaCha20-Poly1305 and refused before any state change | testable, deterministic | — | [default] | `crates/apps/rusty_mux/crates/transport/src/lib.rs` module doc |

## Windows console host (`rusty_conhost`)

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D50 | Build our own in-process console host (option C) | own the stack; ConPTY bugs we can't fix | inbox ConPTY; bundling conpty.dll (#1220); separate exe (fallback only) | [stated] | † conhost/design.md D1: "Author-reported: Nano in the project thread, 2026-10-29 ("D1. Approved.")" |
| D51 | `rusty_conhost` is the Windows default (rusty_term `[console] host`, `RUSTY_CONSOLE_HOST` for mux server and yirp); ConPTY is rollback only | — | ConPTY default | [stated] | primary: Nano, harvest prompt 2026-11-03: "rusty_conhost is the default since #1222; ConPTY is rollback only". #1403 |
| D52 | No silent fallback: an unusable ConDrv is an error; `conpty` is explicit; review the ConPTY path one year after Phase B | a fallback would hide which host runs | auto-fallback | [default] | design §9.2; mem_adb85ef05548436bb3ef657dfd5dfa7f |
| D53 | Phase B criteria: parity suite, manual gate green on two builds, throughput within 10% of ConPTY; a second machine (Lap) sufficed | — | — | [default] | design D5 ("author-reported", no quote); mem_566eaa31cc35495fa5eb8bad181862fe (paraphrase) |
| D54 | #1220 (ConPTY passthrough) closed as superseded | own host passes ?2027h through | ConPTY passthrough | [stated] | † mem_957edaaee0d44b3c825b25c21e4aa2cf: closed 'on Nano's "yes"' |
| D55 | The divergences V1–V15 in design §5.10 are deliberate; the parity suite expects them | each has a recorded why | matching conhost | [default] | design §5.10 |
| D56 | WriteConsoleInput has no cap (conhost parity); terminal input stays capped at 4,096 records by dropping the oldest; V16 withdrawn | parity | a bounded client queue | [default] | mem_523adf7185c14596afca5fa5fe0842f4 (card pick, no quote); #1340 |
| D57 | rusty_term waits up to 5 s on exit so the host's 5 s kill of leftover clients runs | matches Windows' console close | — | [default] | mem_018eb6938591458abcc33e6bbf9e2695 (paraphrase); #1286 |
| D58 | Clean-room contamination is not a concern for rusty_conhost (open-source material only); licence compatibility of ported code still matters | MIT/Apache repo | a clean-room wall | [default] | mem_36bea0ec77a34f719dd6f5960b4de968 (paraphrase). Scope conflict with the mux clean-room rule: Q3 |
| D59 | Windows tests: PR CI runs Windows clippy only; a successful manual Windows workflow run on the exact head before review; Windows tests run in the merge queue | — | — | [default] | design D4 (Dottius); mux handoff §2 |

## GUI (code behaviour only; design goes to D7)

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D60 | Zoom scales only terminal text; Ctrl+Shift+V pastes everywhere, plain Ctrl+V too on Windows; Windows Ctrl+C copies when a selection exists, else sends ^C | Windows Terminal habits | scaling the chrome | [default] | code beats memory: #1388, #1402, #1411; crate README |
| D61 | The frame-time benchmark is a local tool, never a CI gate | wall-clock unstable on runners | gating on frame time | [default] | NOTES D17 ("Unconfirmed"); HANDOFF §6 |
