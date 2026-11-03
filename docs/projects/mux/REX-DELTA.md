# Rex delta check, 2026-10-31

What Rex's public material says on 2026-10-31, against the 2026-10-27 [gap check](REX-GAP-CHECK.md) (G1–G29) and rusty_mux on `main` (2478d63c). Capability parity, not protocol compatibility. Clean room: everything is paraphrased from public docs and write-ups; no Rex code, schema or text is copied. The owner adopted §4 on 2026-10-31: see C25–C27, C35 and C37–C44 in [CAPABILITIES.md](CAPABILITIES.md), Phase 3 and Phase 3b in [ROADMAP.md](ROADMAP.md), and [DECISIONS.md](DECISIONS.md). The owner calls in §4 item 4 stay open.

Tags: **[docs]** a page under `superlogical.com/rex/docs/`, **[cm]** the Classmethod hands-on write-up (tested 2026-10-30), **[news]** third-party reporting, **[unverified]** single source or a search snippet.

## 1. Rex state of play

- **Beta.** macOS public beta since 2026-10-05, invites in waves [superlogical.com/updates/public-testing-beginning]. iOS TestFlight opened to everyone invited; Linux + FreeBSD server testing at about 2,000 people [unverified: Hashimoto on X, snippet only]. Builds seen 2026-10-30: macOS 0.1.0 (1642), server/CLI `rex 0.0.1793299046-g2307b7`, iOS 0.1.0 (1577) [cm]. Linux and Windows clients still not in testing.
- **Still free, no account, self-hostable**; commercial plan to be announced before stable. Open source not announced; a multiplexer beta and "any open-source releases" are promised together [updates post].
- **Docs grew** since 10-27: FreeBSD and Alpine servers, NixOS module, systemd socket activation, key sequences and key modes pages, an agent-skill page, Lua scripts, and nav entries for "Block Plugins" and "Resources and Limits" with no content yet [docs: index].

## 2. What is new or newly documented in Rex

| # | Rex capability | Source |
|---|---|---|
| R1 | Agent skill installer for **four agents**: Claude Code, Codex, OpenCode, Pi; `--project` (per repo), `--dry-run`, `--force`, `--session-naming` (agent names the session after its task); status per item: current, outdated, missing, foreign, unreadable, unsupported. The skill body points at `rex skill show`, so upgrading Rex updates the agent's guide without rerunning setup | docs: automate/agent-skill; cm |
| R2 | More terminal block methods: `size`, `write`, `reset`, `resize`, `set_theme`, `program_status`, `list_dir` (plus format, process, pwd, title) | docs: reference/lua |
| R3 | Layout operations beyond split/zoom: resize a pane, swap blocks, move a block into a new split beside another, detach a block from the layout without closing it (it stays listed as detached), reorder windows in the tab order, server-side focus (direction, next/previous pane and window), floating layers (new, float a block, resize, raise, lower) | docs: reference/lua |
| R4 | `rex wait` on windows and sessions, not only blocks; `--for` timeout | docs: automate/shell-scripting |
| R5 | Key sequences (`ctrl+b>c`, any length, Escape cancels), key modes, conflict warnings naming both bindings | docs: customize/key-sequences, key-modes |
| R6 | Ask a client to act: list a client's actions (`rex -C iPhone actions`), queue one of its actions from a script; app-changing requests need the app's Remote Control switch | docs: reference/lua, agent-skill; cm |
| R7 | Remote identity: `rex whoami`; a `principal` on every client (`rex client ls`) and on `client_connected`/`client_connection_changed`; built-in userspace Tailscale toggled at runtime (`rex server tailscale on`), Tailscale Serve, tailnet address, plain HTTP only with `--allow-anonymous`. **No SSH transport; SSH-key auth is "planned"** | docs: linux/remote-access; cm |
| R8 | Login server: root process, `accounts.jsonc` maps principals (Tailscale user, tagged device, tailnet member, local uid, anonymous) to accounts; ships empty; root never via `*` or anonymous; PAM does account and session checks only, never passwords; per-account servers survive login-server restart and upgrade; reload closes only revoked connections | docs: linux/login-server, server-modes |
| R9 | Hosts: several URLs per host tried in order; bare hostname probes HTTPS 443, HTTPS 7297, HTTP 7297; `rex.host{}` in config and `rex.server(label)` for per-call targeting; `hosts_changed` event. CLI hosts don't yet sync to the Mac app | docs: linux/remote-access, reference/lua; cm |
| R10 | Server lifecycle and packaging: personal server auto-starts and exits ~10 s idle; systemd/Alpine/FreeBSD/NixOS service units; socket activation | docs: linux/server-modes, remote-access |
| R11 | Version skew: either side may be newer; using a feature the server lacks gives an error naming the Rex version needed; too far apart, the server refuses with "update server or client" | docs: linux/upgrading |
| R12 | Events not in our set: `clipboard_written`, `program_status_removed`, `active_window_changed`, `keymap_changed`, `hosts_changed`, `tailscale_status_changed`, `creator_changed`, `client_connection_changed` | docs: reference/events |
| R13 | Program Status Protocol 0.4 (2026-10-10): the `?` reply may list accepted states and kinds | docs: build/program-status |
| R14 | Client UI: sidebar tabs, blocked-agent dot, 41 themes plus Ghostty/iTerm2 import, contrast correction, copy mode done as an action opening scrollback in a floating layer; unified inbox planned | cm; kevinbpatel.com/blog/rex |
| R15 | Remote feel: typing on a wired LAN "same as local", iOS "hardly perceptible" [cm]. No predictive echo, UDP or roaming is documented anywhere | cm; docs: remote-access |

## 3. rusty_mux against it

**Still ahead or by design**
- **Remote transport (C13).** Rex documents HTTP(S) listeners: the caller-identifying ones go through Tailscale, and a plain HTTP listener needs an explicit `--allow-anonymous` opt-in (for use behind a VPN such as ZeroTier). We have SSH bootstrap (#1236), bridge fallback (#1239), make-before-break reconnect (#1243), predictive echo (#1247, #1258, #1267) and our own UDP roaming (#1270, #1280, #1283). Rex has no SSH at all yet.
- Resumable event stream (G7), MCP control server (C07), public Windows server (C24).

**Gaps** (status: *new* = not in CAPABILITIES; *extends* = adds to an existing row)

| # | Gap | Status | Fits | Size |
|---|---|---|---|---|
| D1 | Agent setup for Codex, OpenCode, Pi; `--project`, `--dry-run`; per-item status states; skill text read from the installed binary | extends C34 (Claude Code only today, `ctl/src/agent.rs`) | 2b follow-up | S |
| D2 | Block methods `reset`, `set_theme`, `list_dir`, `size` (we have input, resize, format, process, pwd, title, status, send-keys) | extends C06; `list_dir` is C08's directory browse | 2b follow-up / Phase 4 | S |
| D3 | Resize splits, swap blocks, move a block into a new split, detach a block from the layout while it keeps running, reorder windows, server-side focus | extends C12 (resize explicitly not built) | 2b follow-up | M |
| D4 | Floating layers | new | Phase 4 (client-visible) | M–L |
| D5 | Wait on a window or session | extends C30 (block and event only, `ctl/src/wait.rs`) | 2b follow-up | XS |
| D6 | Key sequences and key modes | extends C19 (G15 still partial) | Phase 4 with clients | M |
| D7 | List and invoke a client's actions | G20 remainder, new | Phase 4 | M |
| D8 | Principal on each client, `whoami`, `client-connection-changed` | extends C25/C27 | **Phase 3 now** | S |
| D9 | Principal→account map with empty default, no root via wildcard or anonymous, PAM account/session checks, per-account servers outliving the front door, reload that closes only revoked connections | shapes C25/C26 | **Phase 3 now** | L |
| D10 | Hosts with several addresses tried in order; per-call server; `hosts-changed` | shapes C35 | **Phase 3 now** | S–M |
| D11 | Daemon lifecycle: auto-start, idle exit, service units, socket activation; FreeBSD as a server target | G1 still partial (C01); FreeBSD new | Phase 5 / owner call | M |
| D12 | Skew errors naming the version needed; refusal when too old | new (we have feature bits, no versioned message) | Phase 5 with C36 | S |
| D13 | Events: clipboard written, status removed, active window changed, config reloaded | extends C29 (none present in `proto/src`) | 2b follow-up | S |
| D14 | Scrollback ~64 MiB / ~100k lines vs our 10,000 rows | G11 note, unchanged | Phase 5 (C22/C28 memory work) | — |
| D15 | PSP 0.4 reply listing states and kinds; our reply is a bare `?` (`rusty_term/core/src/osc.rs`), which 0.4 still allows | conformant; optional | tiny | XS |

## 4. Proposals (adopted 2026-10-31, except item 4)

1. **Phase 3, now:** fold D8, D9 and D10 into the identity steps. D9 is the shape of C26 login mode; with our SSH path the principal is the SSH user, which Rex can't offer yet. D8 and D10 are small and unblock C35.
2. **Phase 3b, a small Rex parity train after Phase 3 identity:** D5, D13, D1, D2 (minus `list_dir`), then D3 (C37–C41). All extend done rows; no new crates, no deps.
3. **Phase 4:** D4, D6, D7 (C42–C44), `list_dir` with C08, sidebar/indicator work under C21.
4. **Owner calls, still open:** whether FreeBSD becomes a server target (D11); whether to raise the scrollback budget (D14); whether D12 lands with C36.

Sources: superlogical.com/rex/docs (index, automate/cli, automate/agent-skill, automate/server-api, automate/shell-scripting, reference/lua, reference/events, build/program-status, customize/key-sequences, linux/server-modes, shared-server, login-server, remote-access, upgrading), superlogical.com/updates/public-testing-beginning, dev.classmethod.jp/en/articles/reona-superlogical-rex-beta-try/, kevinbpatel.com/blog/rex, kucoin.com news flash 2026-10-07, x.com/mitchellh/status/2115569789574033436 (snippet only).
