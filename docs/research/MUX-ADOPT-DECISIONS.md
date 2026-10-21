# Terminal multiplexer: adopt / fork / build decisions

Date: 2026-10-21 · Status: **evaluation only, awaiting owner sign-off** · Base: `main` @ `2c46b57d` · phux: `no-phux/phux` @ `69d7904c` (v0.110.0)

Answers §5 and spikes 1-3 of the design brief "own terminal multiplexer with full Rex capability parity" (owner upload, not in the repo). No implementation code. Clean-room rule held: Rex is referenced only through the brief's public-docs summary; phux was read, not copied.

## 1. Recommendation

**Option 3, build fresh on rusty_mill's own crates, borrowing phux's protocol lessons.** Not a fork (option 1), not a carve-out (option 2).

The brief framed option 3 as "write everything", but most of the target architecture already exists in this repo and has Windows behind it:

| Need | Already in rusty_mill | Windows status |
|---|---|---|
| PTY trait + ConPTY | `platform::pty::{Pty, PtyMaster}` (rustils) with Linux and Windows backends and a mock | consumed by `sessionmgr-pty`; sessionmgr suite 105/105 on real `x86_64-pc-windows-msvc` (`crates/apps/rusty_yirp/README.md`) |
| VT engine | `rusty_term` core: parser, grid, scrollback, OSC, kitty/sixel (48k LOC crate) | ConPTY backend run on Windows 11 build 26200 (`crates/libs/ui/rusty_term/README.md`) |
| Sessions surviving the manager | `sessionmgr-daemon` + `sessionmgr-proc`: detached worker per session, adoption after `SIGKILL` of the daemon, PID + start-time fingerprint | verified on Windows (same README) |
| MCP | `rusty_mcp_server` + `rusty_mcp_proto`, sovereign (no rmcp/rustls/reqwest) | portable |
| Local auth, TLS, AEAD, wire cursor | `rusty_tokio` `UnixStream::peer_cred`, `rusty_tls`, `rusty_aead`, `rusty_wire` | AF_UNIX on Windows in `rusty_tokio` |

Deciding evidence against options 1 and 2 (details in §3):

1. **phux's Windows gap is structural, not a port.** `cargo check -p phux-server` for Windows never reaches phux-server: four of its dependencies fail first (54 errors). phux-server itself has 326 Unix API use sites in 62 files against about 45 `cfg` gates, and its hot upgrade (re-exec + PTY fd adoption) and parking (`fd_shrink.c`) have no ConPTY equivalent.
2. **phux's "portable core" is not portable in the way that matters.** `phux-core` and `phux-protocol` compile for Windows. But the Rex-style binary snapshot bootstrap is a **fork-only libghostty API**: the personal fork exposes 207 FFI functions vs 173 upstream, and the 34 extra include all of `ghostty_snapshot_*`, `ghostty_search_*` and `ghostty_terminal_continuation_*`. Carving the core means inheriting the fork, which the brief forbids without approval.
3. **Two terminal engines would violate the reuse rule.** libghostty-vt would sit beside `rusty_term`, which already has a ConPTY-verified engine. A from-source libghostty build pins Zig (0.16 for the fork, 0.15.2 for upstream) and resolves a 34 to 35 package Zig graph (fonts, sentry, gtk among them) just to emit `lib-vt`.
4. **Hidden egress.** The TUI runs a background update check on every attach unless `PHUX_NO_UPDATE_CHECK` is set (`crates/phux-tui/src/attach/update_notice.rs:1-7`). `phux-client-runtime`, one of the "portable" crates, carries a third-party AI endpoint (`crates/phux-client-runtime/src/organize/mod.rs:34`, `https://api.typesafe.ai/v1/systemone`, key-gated). That fails the sovereignty posture as shipped.
5. **Size.** phux is 627k lines of Rust under `crates/` today (the brief's 525k is stale). The brief's ~95k "portable layers" now measure ~119k (core 8.5k, protocol 39.5k, client-core 30.9k, client-runtime 40.4k), and client-runtime pulls quinn, rustls, tokio-tungstenite, rcgen and minisign-verify.

What we take from phux is ideas, studied and re-written: framing (u32 BE length + u8 type + tagged varint fields, skip unknown, feature bits), `APPLY_INPUT` op-id dedupe with explicit "delivery unknown", generations + tombstones + bounded resync, journaled resumable events, the kind catalog as authz table, and its OSC 7501 record store (`crates/phux-server/src/resource/terminal/program_status.rs`) as a reference reading of the public spec.

**Main risk of this recommendation:** `rusty_term`'s fidelity against libghostty is unmeasured. The `term` port (§8 of the brief) keeps libghostty-vt as a possible second adapter. It should only be added if a measured fidelity gap appears, and then from upstream, not the fork.

### Prerequisites that need owner approval (moves, not new code)

1. **Split `rusty_term` core into a headless crate without tokio.** Today tokio is a hard dependency (`crates/libs/ui/rusty_term/Cargo.toml`, "It is a hard dependency rather than a feature"). The frozen `nexus-vt` already exists only because there was no headless split (CONSOLIDATION-AUDIT §3 row nexus-vt). Both server and clients need the core alone.
2. **Move `sessionmgr-proc` (and possibly the worker/adoption pattern) from `apps/` to `libs/`** if the mux reuses it. ADR-0003 forbids app-to-app edges (`docs/adr/0003-workspace-layout-by-layer.md:77`, "Nothing outside a family depends on an app crate").
3. **Converge the two ConPTY wrappers.** `rusty_term/src/backend/windows.rs` (715 LOC, own `windows-sys` calls) duplicates `platform-windows/src/{pty,sys/pty}.rs` (934 LOC). The mux should use rustils only. Moving `rusty_term` onto it is a separate consolidation PR.

## 2. Adopt table

Paths are relative to `crates/` unless prefixed. "Read" means I opened the file, not only grepped it. LOC counts all `.rs` files (`wc -l`), not code-only. "Dependents" are other workspace manifests naming the crate.

| Component | Candidate | License | Windows? | Size (LOC) | Maintenance | Fit with Nano rules | Decision | Why |
|---|---|---|---|---|---|---|---|---|
| `core` (session/window/block/client, input lease, events, IDs) | **build**; reference `sessionmgr-core` (pure domain, zero I/O, `apps/rusty_yirp/README.md`) and `phux-core` (8.5k, compiles for Windows; deps thiserror, slotmap, serde); IDs from `rusty_uuid` | MIT (sessionmgr) / Apache-2.0 (phux) | yes (pure) | new, target <3k | in-repo | ports-and-adapters, no I/O | **build** | Vocabulary differs from both (`session/window/block/client` vs sessionmgr's agent sessions). Small enough to write; copying phux-core would import slotmap plus phux vocabulary. |
| `pty` | **`platform::pty`** (`platform/rustils/crates/platform/src/pty.rs`, read) + `platform-linux/src/pty.rs` (97) + `platform-windows/src/{pty,sys/pty}.rs` (934); alternative `rusty_term/src/backend/` (1,440) | MIT | **yes, ConPTY**, verified through sessionmgr | 2,002 (api crate) + backends | rustils RFC v2 consumer gate; real consumer `sessionmgr-pty` | object-safe trait, mock backend, `AsRawFd` escape on Linux for a reactor | **adopt** | Exactly the §8 `pty` port, already used. Gaps: **no macOS** (out of scope per its doc comment, `platform-bsd` has no pty) and no resize notification. Extend rustils if macOS is wanted; do not add `portable-pty`. |
| `term` (VT engine behind a trait) | **`rusty_term` core** (`libs/ui/rusty_term/src/core/`, grid.rs 4,969 + parser.rs 1,579, read in part); alternative libghostty-vt (upstream crates.io 0.2.2 or phux's fork, external) | MIT / MIT-or-Apache (libghostty-rs) | rusty_term: yes, Windows 11 verified. libghostty: fork prebuilt only for linux-gnu/aarch64 + macOS; Windows/musl need Zig from source (spike 3) | 48,267 (whole crate) | rusty_term in-repo, 2 dependents (`mill-term`, `rusty_boot`); fork has one author | first-party, no Zig, no C | **adopt + extend** (split headless core) | Second engine forbidden by reuse rule unless measured need. Missing: headless crate without tokio; full-state serializer for snapshots (only viewport repaint exists, `src/render.rs` `render_once`); scrollback replay. |
| `proto` (framing, feature bits, resumable events) | **build** on `rusty_wire` (`foundation/rusty_wire`, 396, Tier S, 9 dependents); patterns from `phux-protocol` (39.5k; deps serde, serde_json, flate2, bytes, bitflags, int-enum); `sessionmgr-protocol` (657, serde JSON lines) as counter-example | MIT-or-Apache / Apache-2.0 | yes | new, target <4k | in-repo | sovereign; no serde in the wire | **build** (extend `rusty_wire` with LEB128 varint) | `rusty_wire` has endian-explicit read/write but no varint (`src/lib.rs` read). Compression deferred behind a feature bit: `rusty_compress` is stored-block only (`foundation/rusty_compress/src/lib.rs:3-7`). |
| `transport` UDS | tokio `UnixStream`, or `rusty_tokio::io::UnixStream` (AF_UNIX also on Windows; `peer_cred` at `libs/async/rusty_tokio/src/io/unix.rs:818`) | MIT | rusty_tokio: yes (`tests/unix_windows.rs`); tokio: Unix only, named pipes on Windows | n/a | rusty_tokio README: "exists to actually understand how an async runtime works, not to replace tokio" | one runtime per binary | **adopt tokio** (owner may overrule) | `rusty_term` is hard-wired to tokio; two runtimes in one binary is worse than named pipes on Windows. Revisit if the owner prefers `rusty_tokio` for sovereignty. |
| `transport` TCP/TLS | **`rusty_tls`** (`libs/net/rusty_tls`, 55,994 incl. hand-rolled engine; Tier A rustls default per ADR-0002) | per crate | yes | 55,994 | in-repo, **do not edit** | ADR-0002 requires it over rustls | **adopt as is** | Brief §8 and ADR-0002 Amendment 1 rule. phux uses rustls 0.23 + ring directly, which is another reason not to carve its client runtime. |
| `transport` QUIC/UDP roaming | none in repo (`grep -rli quic` over manifests: only frozen Nexus shell); phux uses quinn 0.11; `quictun` 8k (Apache-2.0) | n/a | quinn yes | n/a | n/a | new third-party dep needs justification | **defer to spike 4** | Least-specified area. Decide UDP-own vs quinn only after spike 4. quinn would be the first QUIC dependency in the workspace and pulls rustls; check whether `rusty_tls` can back it first. |
| `transport` WebSocket (web client) | `rusty_term/src/web_bridge/{mod,ws}.rs` (921, hand-rolled RFC 6455, `rusty_sha1`) | MIT | yes | 921 | in-repo, feature `web-bridge` | zero new deps | **adopt pattern / extract later** | One call site today. Extract to a shared crate only when the mux becomes the second (no abstraction before two call sites). |
| `auth` local peer | `rusty_tokio` `peer_cred` (above); tokio has `peer_cred` on Unix; Windows named pipe needs `GetNamedPipeClientProcessId` (build) | MIT | partial | small | in-repo | validate at boundary | **adopt + build Windows shim** | Small, platform-specific; belongs in rustils `platform` if it grows a second consumer. |
| `auth` Tailscale identity | `rusty_tailscale` (`apps/rusty_tailscale`, 15 crates, ~11k; `README.md` read). `ts-localapi` serves only status/prefs/ping (`crates/ts-localapi/src/lib.rs:381-395`), **no whois** | per crate (unverified) | unverified | ~11k | in-repo, Phase 7 | app layer: cannot be a Cargo dependency (ADR-0003) | **extend `ts-localapi` with `whois`; consume over HTTP-on-UDS** | Talking to LocalAPI over UDS with `rusty_http` needs no Cargo edge and works against real tailscaled too. `ts-net` (embedded tailnet) is tokio + smoltcp and would need a libs move first. |
| `auth` PAM / login mode | none in repo (grep `pam` outside Nexus: 0 hits) | n/a | Linux only | n/a | n/a | FFI to libpam is a new system dep | **build (phase 3)** | Thin FFI behind a trait; Linux-only. |
| `auth` scoped credentials, constant-time compare | `rusty_crypto_key::constant_time_eq` (CONSOLIDATION-AUDIT row 1, done), `rusty_oauth::bearer` | MIT-or-Apache | yes | small | in-repo | no new crypto | **adopt** | Rule: no second crypto implementation. |
| `server` daemon, survival, adoption | `sessionmgr-daemon` (8,558) + `sessionmgr-proc` (769, `src/lib.rs` header read): detached worker, PID + start-time fingerprint | MIT | **yes, verified** | 9,327 | in-repo, app layer | proven pattern | **reuse pattern; move `sessionmgr-proc` to libs if imported** | Solves "sessions outlive the manager" on Windows without Job Objects. The mux server is one process owning many PTYs, not a worker per session; copy the design, not the daemon. |
| `server` hot upgrade | phux re-exec + PTY fd adoption (`phux-server/src/runtime/upgrade.rs`, 18 Unix sites; `portable-pty-adopt`) | Apache-2.0 | **no**: ConPTY handles cannot be adopted by a new process the same way (unverified, needs spike 1 box) | n/a | n/a | n/a | **build later (Unix first)**; on Windows, use sessionmgr's worker-per-session survival instead | Structural difference; document per-platform. |
| `server` parking, encryption at rest | `rusty_aead` (ChaCha20-Poly1305, 2,495, "not proven, not independently reviewed", `src/lib.rs`); key storage `platform::security::CredentialStore` (`platform/rustils/crates/platform/src/security.rs:78`); `rusty_atomic_file` for writes | MIT-or-Apache / MIT | yes | 2,495 | in-repo | no new crypto | **adopt (phase 5)** | Snapshot format needs the `term` serializer first. |
| `client-tui` | **`rusty_term` runtime** (TUI passthrough: parses PTY bytes into a Grid and re-emits to the host terminal, README) vs `sessionmgr-tui` (ratatui, crossterm, vt100, tui-term) | MIT | yes | (in 48k) / 1,973 | in-repo | first-party engine; sessionmgr-tui adds 4 external crates | **adopt `rusty_term`** | It already is a single-pane mux client that owns parse + render. Needs a network byte source in place of a local PTY. |
| `client-web` | `rusty_term/web/` (Vite/TS frontend prototype) + `web_bridge` | MIT | n/a | not counted | prototype | n/a | **adopt prototype; unverified** | Not read beyond `package.json` presence. Whether it parses VT itself (xterm.js or own) is unverified. |
| `client-native` | `rusty_term` `gui` feature (winit + softbuffer/wgpu, `rusty_font`) | MIT | yes, run on Windows | (in 48k) | in-repo | external winit/wgpu behind feature | **adopt (phase 4)** | Exists; no new renderer. |
| `ctl` (CLI from schema, `--json`, exit codes) | no shared CLI crate (CONSOLIDATION-AUDIT §2.4 and Q7: "Leave"); clap in 11 manifests; orch hand-rolls by ADR-0009 | n/a | yes | new | n/a | schema-first | **build** (generated from the schema catalog, hand-rolled parser) | A schema-driven parser is small and keeps exit codes (0/1/2/3/4/130) in one table. |
| `mcp` | **`rusty_mcp_server`** (11,588) on **`rusty_mcp_proto`** (4,782), both read (`src/lib.rs` headers, manifests) | MIT-or-Apache | yes | 16,370 | in-repo, active MCP track | sovereign: depends on `rusty_json`, `rusty_serve`, `rusty_http`, `rusty_oauth`; no rustls/reqwest | **adopt** | Tool registry fits "schema-first API generates CLI + MCP". Do not use `rusty-mcp` (gateway-support remnant per CONSOLIDATION-AUDIT update 2026-10-10). |
| `status` (program status OSC) | `rusty_term/src/core/osc.rs` (OSC dispatch) + `l13` (OSC 5379 JSON-RPC side channel, `l13/src/lib.rs:43`); reference `phux-server/src/resource/terminal/program_status.rs` | MIT / Apache-2.0 | depends on ConPTY OSC pass-through (spike 1) | 1,411 (osc + l13) | in-repo | small parser | **build on `rusty_term` OSC**; interoperate with 7501 v0.4 | The brief's scorecard is stale: phux now implements OSC 7501. l13 pulls `serde_json` + `rusty_lsp`, too heavy for status. |
| `script` (config, bindings, actions) | none: no Lua in the workspace (`grep -rln "mlua\|rlua\|piccolo"` over manifests: 0); `rusty_codec` TOML (764, 2 dependents); `rush` is a shell app (layer apps) | MIT-or-Apache | yes | 764 | in-repo | no new interpreter | **build: TOML config via `rusty_codec`; automation via `ctl`/API; no embedded scripting until phase 2 shows a need** | Lua means a C library (mlua) or a large Rust VM; neither is justified before two real call sites. |
| async runtime (cross-cutting) | tokio (77 manifests, hard dep of `rusty_term`) vs `rusty_tokio` (sessionmgr ADR-0001) | MIT | both | n/a | n/a | async only for real I/O | **tokio**, owner decision | See UDS row. |

Not used and why: `nexus-terminal` / `nexus-vt` (frozen; `nexus-vt` is a stale fork of `rusty_term` core, CONSOLIDATION-AUDIT §3); `rusty_lines` (line editor, not a terminal); `rusty_agui` (agent-to-UI events, not terminal I/O); `rusty_search`, `rusty_multimodal_db`, `rusty_mill_board`, `rusty_homelab_mcp`, `rusty_fedora_agent`, `rusty_whisper`, remind-me (no component in §8 needs them; not opened beyond the consolidation audit).

## 3. Spike reports

All spikes ran in a Linux cloud container, toolchain 1.99.0 for phux and 1.97.0 for the scratch crates, Zig from the PyPI `ziglang` wheels (0.16.0 and 0.15.2) because ziglang.org is blocked here. Scratch work stayed outside the repo.

### Spike 1: ConPTY tee — needs the owner's Windows machine

Not run: no Windows host here, and faking it would be worse than nothing. What the repo already shows:

- Both ConPTY wrappers call `CreatePseudoConsole(size, in, out, 0, &hpc)` with **flags 0** (`rusty_term/src/backend/windows.rs:112`, `platform-windows/src/sys/pty.rs:174`). In this mode ConPTY is itself a renderer and emits its own VT, not the child's bytes.
- `rusty_term` verified that OSC window titles survive ConPTY on Windows 11 (README "Platform support").

Assessment (inference, unverified): the **tee architecture stays viable** on Windows. The server tees whatever ConPTY emits, and its own engine parses the same bytes, so snapshots and clients stay consistent. No server-side re-synthesis is needed for correctness. The open question is **fidelity**: does ConPTY pass through private OSC sequences (OSC 7501 program status, OSC 133 marks, OSC 8 links), sixel/kitty graphics, and does it repaint the whole screen on resize?

Procedure for the owner's box (about 30 minutes): build `sessionmgr-pty`'s example or a 40-line scratch binary on `platform::pty`; run the same script under Linux and Windows (`printf` of SGR, OSC 8, OSC 133 A/B/C/D, OSC 7501 `working` then `done`, a 2,000-line `seq`, then a resize); capture raw bytes; feed both captures to `rusty_term` core; diff (a) the final grid and (b) which OSCs arrived. Pass = identical grid and all OSCs present. If the OSCs are stripped, Windows status signalling needs a side channel (for example a named pipe the child inherits) instead of in-band OSC.

### Spike 2: phux-server on Windows — dependencies fail before the server compiles

Command: `cargo check --keep-going -p phux-server --target x86_64-pc-windows-gnu`.

Setup notes: `-msvc` could not get past `ring`'s C build here (no MSVC CRT headers; clang-cl + llvm-lib + mingw headers do not mix). `-gnu` with `zig cc` as the C compiler works, and `cfg(windows)` Rust code is identical for both ABIs. libghostty-vt-sys's build script was pointed at an empty placeholder library (`LIBGHOSTTY_VT_SYS_LIB_DIR`); `cargo check` never links, so this tests Rust code only.

| Crate | Errors | What fails | Class |
|---|---|---|---|
| `phux-config-core` | 10 | `std::os::unix` paths, `OsStr::as_bytes`, `Metadata::uid`, `OpenOptions::mode`, `nix::unistd` | platform-trait fixable (path encoding, ownership check, file modes → ACLs) |
| `quictun` | 6 | secret-file `mode`/`uid`, `rustix::process` | fixable, but the Windows file-permission story (ACLs) is design work |
| `phux-perf` | 3 | `nix::sys::resource` (getrusage) | fixable |
| `phux-git` | 35 | `O_DIRECTORY`/`O_NOFOLLOW` "beneath" path resolution, `libc::kill`/`SIGKILL` | partly structural: safe "beneath" resolution has no direct Win32 equivalent |
| `phux-server` | **not reached** | depends on the four above | static count: 326 Unix API use sites (`std::os::unix|fd`, `nix::`, `rustix::`, `libc::`) in 62 files, about 45 `cfg` gates. Heaviest: `resource/terminal/park.rs` 50, `spawn.rs` 37, `fd_shrink.rs` 32, `runtime/upgrade.rs` 18, `proc_query.rs` 15, `transport.rs` 14, `auth.rs` 14 |

Portable crates checked separately for Windows: `phux-core` **ok**, `phux-protocol` **ok** (also with `--features server`), `phux-client-core` **blocked only by `phux-perf`** (3 errors), `phux-client-runtime` blocked by `phux-config-core` + `phux-perf`.

Classification: the dependency crates are fixable behind a platform trait (54 errors). phux-server's parking (`fd_shrink.c`), PTY fd adoption for hot upgrade, and `proc_query` are **structural**: they assume fds and `/proc`-style process queries throughout. A Windows phux-server is a rewrite of its terminal resource layer, which is the part rustils + sessionmgr already provide.

### Spike 3: libghostty-vt on Windows and musl, upstream vs fork — partly done

| Variant | Zig | Result here |
|---|---|---|
| Fork (`phall1/libghostty-rs@fd9d9001`, ghostty `phall1/ghostty@11e797c0`), linux-gnu, prebuilt download | none | **builds and runs.** Smoke test (`Terminal::new(80,24)`, write `hello \e[31mred\e[0m\r\nline2`, read cursor) prints `cursor=(5,1)`. Stripped binary 2.50 MB vs 0.34 MB empty: **~2.16 MB** for the engine. Static `.a` 18.9 MB. |
| Fork, `x86_64-unknown-linux-musl`, from source | 0.16.0 | **blocked by this sandbox**: 2 Zig packages unreachable (`uucode@9d55524` mirrored from GitHub ok; `translate-c` lives on codeberg.org, which is denied here). |
| Upstream (crates.io `libghostty-vt 0.2.2`, ghostty `ghostty-org/ghostty@a887df42`), linux-gnu, from source | 0.15.2 (0.16 refused: `minimum_zig_version = "0.15.2"`) | **blocked by this sandbox**: the build resolves a 34-package Zig graph from `deps.files.ghostty.org` (denied here). 7 code packages mirrored from GitHub with matching hashes; it then needs libxml2, imgui, freetype, harfbuzz, sentry and more just to configure `lib-vt`. |
| Either, `x86_64-pc-windows-msvc` | either | not attempted past `cargo check`; producing and linking a `.lib` needs the Windows SDK, so it belongs with spike 1 on the owner's box. |

Findings that hold regardless:

- **FFI surface:** upstream 0.2.2 exports 173 `ghostty_*` functions; the fork 207. Fork-only: `ghostty_snapshot_{capture,decoder,encode}*` (14), `ghostty_search_*` (8), `ghostty_terminal_continuation_*` (3), `vt_write_until_ground`, `paste`, `clear_presentation`, formatter, OSC set, dirty-row iteration. Upstream-only: `render_state_colors_get`, `terminal_mode_{get,set}`. **phux's native-checkpoint bootstrap is not available on upstream libghostty.**
- **Build egress:** both build scripts fetch at build time (prebuilt via `curl` from GitHub releases, or `git clone` of ghostty plus Zig package fetches). An offline build needs `LIBGHOSTTY_VT_SYS_LIB_DIR` or `GHOSTTY_ZIG_SYSTEM_DIR` with a mirrored package set.
- **Toolchain pins diverge:** fork needs Zig 0.16.x, upstream needs 0.15.2.

To finish spike 3: on a machine with open egress, run `cargo build --release --target x86_64-unknown-linux-musl` and `--target x86_64-pc-windows-msvc` for both variants with `LIBGHOSTTY_VT_SYS_FROM_SOURCE=1`, recording time and `.a`/`.lib` size. Only worth doing if the owner wants libghostty kept as a second `term` adapter.

## 4. Corrections to the brief

- phux size: 627k lines under `crates/` (720k repo-wide), not 525k. Portable layers ~119k, not ~95k.
- phux now implements OSC 7501 (`crates/phux-protocol/src/wire/frame/mod.rs:449`, `phux-server/src/resource/terminal/program_status.rs`): "program status: agent state only, no OSC 7501" is stale.
- `rusty_tail` is `crates/apps/rusty_tailscale` (README title "tailscale-rs (rusty_tail)"), app layer.
- `rusty-mcp` is no longer the MCP server crate. The sovereign stack is `rusty_mcp_proto` + `rusty_mcp_server` (+ `rusty_mcp_client_native`, `rusty_mcp_axum`).
- `phux-protocol`, `phux-client-core` and `phux-client-runtime` make libghostty optional (features `render-pool`/`server`, `native-engine`, `engine`), so the codec builds without Zig. The checkpoint path still needs the fork.

## 5. Unverified / not done

- Spike 1 entirely; spike 3 from-source builds; spikes 4-5 (out of scope for this pass).
- `rusty_term` web frontend internals; `rusty_tailscale` crate licenses; `rusty_tokio` Windows AF_UNIX peer identity semantics.
- `rusty_term` vs libghostty fidelity on real workloads (vttest, tmux-style apps, wide/RTL text).
- phux egress grep covered `https?://` literals and `curl` callers in Rust; build scripts, JS/TS clients and Nix files were not audited.
