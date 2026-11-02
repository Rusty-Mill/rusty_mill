# Rex app check, 2026-11-01

Built 2026-11-01 from a copy of the macOS `Rex.dmg` (0.1.0, build 1696) and Superlogical's public docs. It follows the [Rex delta check](REX-DELTA.md). This is a clean-room read. I looked at the bundle layout, property lists, resource files, linked libraries, Go package and source-file **names**, and short user-facing strings inside the binaries. I did not disassemble or decompile anything, and no Rex code is reproduced here. Wording is paraphrased, apart from identifiers such as command, method, event and action names.

**Artifact identity.** SHA-256 of the files read, from the image's HFS+ volume:
- `Rex Beta.app/Contents/Info.plist`: `c5e721597ca715450b69ecc37cc4cd93d11062d220a63c8ac35f9ef7388512ff`
- `Contents/MacOS/Rex`: `a243a535a15c6083eff1d9655b7864b200a4a822691bf395aba7ffd3d51bdd07`
- `Contents/Helpers/Rex Server.rex/Contents/MacOS/rex`: `524cfc1eb9a271d9285975f392d9f943e7179142c5749741cb1dd03fed4cf282`

**How strong the evidence is.** Rex was never run. A **[pkg]** or **[str]** tag shows only that a package, file name or string is in the bundle; the behaviour a row describes is **inferred** from that name and is not verified. It may be partial, disabled, or work differently from what the name suggests. Only **[docs]** and **[cm]** describe behaviour someone observed or Superlogical states. Rows marked *(inferred)* rest on names and strings alone and carry no guarantee; a row adopted from one is a direction for rusty_mux, not a claim about Rex.

**Evidence tags**
- **[plist]**: `Rex Beta.app/Contents/Info.plist`, or a bundle's own Info.plist.
- **[bundle]**: a file or folder in the app bundle.
- **[pkg]**: a Go package or source-file path compiled into the server binary (`Helpers/Rex Server.rex/Contents/MacOS/rex`), such as `internal/loginserver/pam_checker.go`.
- **[str]**: an identifier or interface string in a binary. `app` is the Swift app (`MacOS/Rex`), `srv` is the Go server.
- **[docs]**: a page under `superlogical.com/rex/docs/`.
- **[cm]**: the Classmethod hands-on write-up of 2026-10-31.

Status against rusty_mux: **have** (done), **part** (partly done), **plan** (on the roadmap as a capability row), **new** (not on the roadmap). Capability ids are from [CAPABILITIES.md](CAPABILITIES.md); statuses are as of 2026-11-01, before the rows in §3 were added.

---

## 1. How Rex.dmg works

| Fact | Evidence |
|---|---|
| The DMG is a UDIF image whose chunks are LZMA (xz) compressed. It holds a GPT disk with one HFS+ volume named "Rex Beta". The volume has a Finder background image and the app. 7-Zip 23 writes zeros for xz chunks, so I decoded the chunks myself and then extracted the volume | koly trailer; blkx chunk type `0x80000008` |
| App: `Rex Beta.app`, bundle id `com.superlogical.rex`, version 0.1.0 (build 1696), release channel `preview`, flavor `production`. It records its source commit. Built with Xcode 27 against the macOS 27 SDK; the minimum is macOS 15 | [plist] |
| Universal binaries (x86_64 and arm64) for both the app and the server | `file` |
| **Two programs.** `MacOS/Rex` (60 MB) is the **Swift** GUI client. `Helpers/Rex Server.rex/Contents/MacOS/rex` is a separate **Go** binary (built with go1.27.3). It is both the `rex` CLI and the server, and runs under its own bundle id `com.superlogical.rex.server` with the name "Rex Server" | [plist] `RexServerBundleIdentifier`, `RexServerName`; [str srv] `go1.27.3`, `go.superlogical.com/rex/cmd/rex` |
| The GUI renders terminals with **libghostty**. It bundles `swift-ghostty_GhosttyRender.bundle` (a Metal shader library and the Symbols Nerd Font). The server links Go bindings to libghostty (`go.mitchellh.com/libghostty`), so the server keeps real terminal state | [bundle]; [pkg] |
| GUI networking uses SwiftNIO (`swift-nio_NIOPosix.bundle`) | [bundle] |
| Updates come through **Sparkle**: a signed EdDSA appcast at `blob.superlogical.com/public/rex-app/preview/appcast.xml`, automatic checks on, and (inferred from a string) the update verified before it is extracted. Developer Mode offers a "Bleeding Edge" channel built from every commit that passes CI | [plist] `SU*`; `Frameworks/Sparkle.framework`; [str app] |
| The app registers a `rex://` URL scheme and an "Alternate" viewer role for every file and folder (`public.item`), so it can open a path in a terminal | [plist] `CFBundleURLTypes`, `CFBundleDocumentTypes` |
| The app holds macOS privacy prompts **on behalf of the programs in its terminals**: camera, microphone, system audio, Bluetooth, contacts, calendars, photos, location, AppleEvents and local network. Each prompt says that Rex itself does not use the resource. A server method grants file access, and another reports privacy status | [plist] `NS*UsageDescription`; [str srv] `server.apple.grant_file_access`, `server.apple.privacy_status`; [pkg] `internal/tcc`, `internal/sandboxext` |
| The server runs under launchd on macOS and under systemd elsewhere, with socket activation and readiness notification. It takes a process lock, claims its runtime directory and reports readiness through a file descriptor | [pkg] `internal/launchd`, `internal/sdlisten`, `internal/sdnotify`, `server/lifecycle/readiness*.go`, `internal/processlock`; [str] `REX_SERVER_READY_FD` |
| The app can run the bundled server or a server built from a local source checkout (a developer setting), and checks that the two are compatible | [str app] "Choose the included server or run Rex from a source checkout", "Server and app are incompatible" |
| Package map of the server: `internal/{server,session,layout,script,cli,netapi,sessionapi,snapshot,keymap,hosts,home,tailscale,loginserver,pam,sshconfig,openssh,terminfo,agentharness,rexskill,blockplugin,overlay,backlog,procinfo,pty,...}`, `blocks/{terminal,onboarding}`, `sdk/{block,blockserver,blockremote,blockexec,apiclient,apitypes,apihistory,compat}` | [pkg] |

---

## 2. Capability and feature list

### A. Server, sessions and object model

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| A1 | Server-owned sessions outlive clients; the app reconnects ("Sessions stay open while Rex reconnects") | [str app]; [docs] | have (C01 part) |
| A2 | Four kinds of object (session, window, block, client), each with an id and a changeable label | [docs]; [pkg] `session/label.go`, `client/label.go` | have (C11, C33) |
| A3 | *(inferred)* **Sessions survive a server restart**: they are persisted to a SQLite store (generations, sweeping, transactions) and restored | [pkg] `server/persist/{persist,restore,attach}.go`, `session/record_restore.go`, `internal/snapshot/*`; [str] `PRAGMA user_version` | new (beyond C36) |
| A4 | *(inferred)* **Encrypted on-disk terminal state**: sealed files with chunk headers and keys | [pkg] `blocks/terminal/internal/sealfile/*`, `savedstate` | plan (C28) |
| A5 | *(inferred)* Idle-terminal **parking, suspend and compression** inside the terminal block's mux layer | [pkg] `blocks/terminal/internal/mux/{park,suspend,compress,memory,pause}.go`, `terminal/suspend.go`, `server/stream_suspend.go` | plan (C22) |
| A6 | Server-wide limits (such as a session cap) | [pkg] `server/limits.go`, `session/limit.go`; [docs] `--max-sessions` | plan (C23) |
| A7 | *(inferred)* Block **ownership**: own and disown a block | [str] `session.own_block`, `session.disown_block`; [pkg] `terminal/owned.go`, `owner.go`, `session/ownership.go` | new |
| A8 | Detached blocks: a block can live outside any layout, and blocks can be created unplaced | [str] `session.new_block`; [docs] `detach_block`, `list_blocks` | new (C41) |
| A9 | Auto-start on first use; exit when idle | [pkg] `cli/autostart`, `lifecycle/autostart.go`, `activity.go`; [docs] | part (C01) |
| A10 | *(inferred)* Generated adjective-noun session names, and agents that name the session after their task | [pkg] `internal/haiku` (`AdjNoun`); [str] `feature-session-naming`, "Let agents name sessions" | new |

### B. Layout and windows

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| B1 | *(inferred)* A split tree per window on the server; zoom | [pkg] `internal/layout/*`, `session/block_zoom.go` | have (C12) |
| B2 | *(inferred)* Resize panes, autobalance splits, swap and move blocks, move windows, server-side focus by direction or order | [str] `session.resize_pane`, `session.swap_blocks`, `session.move_block`, `session.move_window`, `session.focus_*`; "Autobalance splits" | plan (C41) |
| B3 | *(inferred)* Floating layers: new layer, float a block, resize, raise, lower | [str] `session.new_layer`, `session.float_block`, `session.raise_layer`; [pkg] `session/layer*.go`, `server/layer_store.go` | plan (C42) |
| B4 | *(inferred)* Layouts as data: a layout library, Lua and JSON forms, a text **diagram** of a layout, and a spatial model | [pkg] `layout/{diagram,spatial,layered,resize}.go`; [str] `layout.library`, `layout.json`, `layout.lua` | part |
| B5 | *(inferred)* Move a session between windows, open a session in every window, collapse sessions in the sidebar | [str] `rex session move`; "Collapse Other Sessions" | new |

### C. Terminal block

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| C1 | Methods: `format`, `title`, `pwd`, `process`, `write`, `reset`, `clear`, `resize`, `set_theme`, `program_status`, `list_dir`, `modes`, `shell`, `blob_read` | [str srv] `com.superlogical.terminal.*`; [pkg] `terminal/api_{resize,theme}.go`, `list_dir.go`; [docs] | part (C06, C40) |
| C2 | *(inferred)* **Kitty graphics protocol** support, including shared-memory and file transfer media and APC rewriting through the mux | [pkg] `terminal/internal/kittyapc/*`, `kittymedium/{shm,file_unix}.go`, `mux/kitty_rewrite.go` | new |
| C3 | *(inferred)* Clipboard: OSC 52 writes, observed clipboard, and reads gated by the app ("The copied text expired before Rex could read it") | [pkg] `terminal/clipboard{,_observed,_read}.go`; [str] | new (C38 event only) |
| C4 | *(inferred)* Selection on the server | [pkg] `terminal/selection.go` | new |
| C5 | *(inferred)* Process ancestry and a process-changed event | [pkg] `terminal/{ancestry,process_changed}.go`, `internal/procinfo` | have (C31) |
| C6 | Program status (OSC 7501) | [pkg] `terminal/program_status.go`; [docs] | have (C20) |
| C7 | *(inferred)* Locale and termenv set up per terminal; login-shell detection; terminfo shipped | [pkg] `internal/locale`, `termenv`, `shell/default*.go`, `sysuser/shell_*.go` | part |
| C8 | *(inferred)* Scrollback spool to disk (`dataspool`), backlog writer | [pkg] `terminal/internal/dataspool`, `internal/backlog/writer.go` | part (C03) |
| C9 | *(inferred)* Prompt marks: jump to the previous or next prompt; find in scrollback | [str app] "Jump to Next Prompt", "Find in Scrollback" | new |
| C10 | *(inferred)* A theme the mux holds for a terminal (`mux/theme.go`); whether it outlasts a reset is unknown | [pkg] | part (C40 covers colours programs see) |

### D. Block platform and plugins

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| D1 | *(inferred)* **Block plugins**: blocks run as separate processes with a control channel, configured, loaded and hot-reloaded from source | [pkg] `internal/blockplugin/{config,load,reload,reload_source}.go`, `sdk/{blockserver,blockremote,blockexec}`; [str] "block plugin loaded", "block plugin reloaded", "block plugin process exited" | new |
| D2 | *(inferred)* A public **block SDK** (`sdk/block`, `apitypes`, `apiclient`, `compat`, `fsx`) | [pkg] | new |
| D3 | *(inferred)* A second built-in block type, **onboarding** (`com.superlogical.onboarding`, with `set_step` and `state`) | [pkg] `blocks/onboarding/*`; [str] | new |
| D4 | *(inferred)* Block type discovery: `rex block types` | [pkg] `cli/block_types` | part (C17) |

### E. API, transport and protocol

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| E1 | Self-describing API: `api list`, `describe`, `call` and `help`, with JSON Schema | [pkg] `cli/api_*`; [docs] | have (C17) |
| E2 | *(inferred)* Network API over HTTP with a **WebSocket control channel**, ordered dispatch, a status endpoint and an observer | [pkg] `netapi/control/{dispatch,websocket,status}.go`, `server/http/control*.go`, `httptransport/websocket.go` | new (we use our own wire) |
| E3 | *(inferred)* Separate data streams with **tickets**, a nonce handshake and policy rejects; reconnect tokens and resume credentials | [str app] "data ticket", "data WELCOME nonce", "reconnect token", "resume credential" | have (C13 resume) |
| E4 | *(inferred)* API revisions are negotiated ("The server supports API revisions…"); clear errors when app and server differ, naming what is needed | [str app]; [pkg] `sdk/compat` | plan (D12) |
| E5 | *(inferred)* Call history: recording of API calls for diagnostics | [pkg] `sdk/apihistory`, `server/control_history.go` | new |
| E6 | *(inferred)* **LLM-readable help served by the server** (llms-style markdown generated from the API) | [pkg] `server/http/{help,llms_fields}.go`; [str] `llmsServer`, `llmsTemplateDoc` | new |
| E7 | Event stream: `rex events`, plus events for sessions, windows, blocks, clients, keymap, hosts and Tailscale | [docs]; [pkg] `session/event_dispatch.go` | have (C05), part (C38) |

### F. Identity, remote and multi-server

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| F1 | *(inferred)* **Embedded Tailscale (tsnet)**: join, log out, rename, tags, machine approval, MTU, serve listener, admitting tailnet peers; `whoami` | [pkg] `internal/tailscale/*`, `tailscaled/admit.go`; [str] `server.tailscale.enable`, `server.tailscale.logout`, `server.whoami` | plan (C27) |
| F2 | *(inferred)* **Pair a device with a QR code** (the iOS client) | [pkg] `cli/server_tailscale_pair`; [str app] "Show pairing QR code" | new |
| F3 | *(inferred)* Login server: an account map, forwarded identity, a policy, PAM account checks | [pkg] `internal/loginserver/*`, `internal/pam` | plan (C26) |
| F4 | *(inferred)* Identity principals and selectors on every connection | [pkg] `server/identity/{principal,selector}.go` | plan (C25) |
| F5 | *(inferred)* Multiple listeners, an anonymous listener behind an opt-in, and identifying peers by tcp or unix credentials | [pkg] `internal/multilistener`, `lifecycle/listener_anonymous.go`, `internal/tcppeer` | part |
| F6 | *(inferred)* **SSH awareness**: reads and edits `~/.ssh/config` (includes, destinations, verification, diagnosis, setup), checks the OpenSSH version | [pkg] `internal/sshconfig/{setup,verify,diagnose,include,destination}.go`, `internal/openssh/version.go` | ahead (we have the SSH transport, #1236) |
| F7 | *(inferred)* **Installs Rex's terminfo on remote hosts** (`rex terminfo setup`, `status`, `host`, `remove`, `show`) | [pkg] `cli/terminfo*`, `internal/terminfo` | new |
| F8 | *(inferred)* Saved hosts that can be **declared in config** (`rex.host{}`), a server per call, `hosts reaches`, a home server (`rex home set`), a durable host id, and a sync of hosts between machines ("RexSync") | [pkg] `internal/hosts/declared.go`, `internal/home/*`, `internal/hostid`; [str] `com.superlogical.rex.sync`, "RexSync…" | plan (C35), new (sync) |
| F9 | *(inferred)* A **hub** in the app: one window shows sessions from several servers, with diagnostics | [str app] "Inspect the hub's servers and sessions" | new (C16) |
| F10 | *(inferred)* Removing a host leaves its sessions running remotely | [str app] | have (by design) |

### G. Automation, scripting and agents

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| G1 | *(inferred)* CLI covering sessions, windows, blocks, clients, focus, swap, move, zoom, resize, send, send-key, capture, wait, run, kill, events, `on`, do, actions, keymap, config, api, hosts, home, skill, terminfo, server, login-server, whoami, version, and completion for bash, zsh, fish, nu and PowerShell | [pkg] `internal/cli/*` (85 command files) | part (C04) |
| G2 | Lua 5.1 scripting: actions, bindings, modes, timers, exec, waits, `rex.on` events, cross-server calls, JSON | [pkg] `internal/script/*` (timer, exec, luajson, hostdecl, key_sequences); [docs] | by decision (C19 declarative) |
| G3 | *(inferred)* Config overlay: settings recorded in the app layer over the config file and take priority ("Recorded here. It takes priority over your configuration file") | [pkg] `internal/overlay/*`; [str app] | new |
| G4 | *(inferred)* **Agent harness**: install a skill and hooks for Claude Code, Codex, OpenCode and Pi, with plan, item status, and hook-file formats | [pkg] `internal/agentharness/{known/{claude,codex,opencode,pi},plan,hookformat,skillfile}.go`, `internal/rexskill/*` | have (C34 Claude), plan (C39) |
| G5 | *(inferred)* Agents name their session after their task (opt-in) | [str app] "Agents label the session after the task they're working on" | new |
| G6 | **Config migration from eight tools** (tmux, zellij, WezTerm, kitty, Alacritty, Ghostty, iTerm2, herdr). Rex reads the other tool's files, translates and verifies each key binding, then writes a prompt for a coding agent to apply them to `init.lua` (a fast path when everything is pre-translated). Ghostty themes and fonts are migrated directly | [bundle] `RexApp_RexMigrate.bundle/.../Sections/*.md`, `prompt.md.tmpl`; [str app] "Migrate your config to Rex", "Paste this prompt into an agent" | new |
| G7 | *(inferred)* Client actions: list and run another client's actions (`client.perform`, `client.queue`, `client.reply`) | [str srv]; [pkg] `server/client_{action,perform}.go` | plan (C44) |

### H. Keys and input

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| H1 | *(inferred)* Key bindings, key sequences, key modes (exclusive and once), conflict detection, a keymap vocabulary with schema references | [pkg] `keymap/{merge,mode,vocabulary,schema_refs}.go`, `script/key_sequences.go` | plan (C43) |
| H2 | *(inferred)* Recording a shortcut in the app, "Copy Shortcuts as Lua", hints for key sequences | [str app] | new (C08) |
| H3 | *(inferred)* Key forwarding and routing between the client and server (replay, resolve, translate) | [str] `client.key.forward`; "Key replay", "Key routing" | part |
| H4 | Named-key sends | [docs] | have (C32) |

### I. Client application (macOS)

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| I1 | *(inferred)* Command palette that searches commands, sessions and tabs | [str app] | plan (C08) |
| I2 | *(inferred)* Sidebar with sessions; tabs above or on the left or right; pane headers; up to five pane icons per tab; showing each process in tabs | [str app] | plan (C21) |
| I3 | *(inferred)* **Program activity**: per-status indicators (done, busy, needs you, error), Dock badge, sounds, system notifications, an "Action center" | [str app] "Shown when a program needs you to respond", "Badge Dock icon", "Play sounds", "Action center" | plan (C21) |
| I4 | *(inferred)* OSC 9 and OSC 777 desktop notifications, with floating-notification placement | [str app] | have (C29 event), client new |
| I5 | Themes: 41 built in, light and dark palettes, import of Ghostty and iTerm themes, a theme customizer, contrast correction, Display P3, an extended palette | [bundle] `themes.json` (41 names); [str app] | new (C16) |
| I6 | Interface materials (System, Sequoia, Modern, UseTheme, BlendTheme), "Glass" panes, comfortable or compact pane styles | [bundle] `MaterialStyles-*.json`; [str app] | new |
| I7 | *(inferred)* Fonts: fallback chain, weight, ligatures, character spacing, line height | [str app] | new |
| I8 | *(inferred)* **Custom shaders** (Shadertoy-style `mainImage`) | [str app] "The shader doesn't define mainImage" | new |
| I9 | *(inferred)* Focus follows mouse, fading inactive panes, an animated focus highlight, trackpad gestures (pinch to zoom panes, swipe for the sidebar) | [str app] | new |
| I10 | *(inferred)* Link safety: links that open other apps ask first, with a list of trusted origins | [str app] "Links that open other apps need confirmation", "Review the permitted origins" | new |
| I11 | *(inferred)* Insert Path and Go to Directory (recent paths, folder listing) | [str app]; [str srv] `list_dir` | plan (C08) |
| I12 | *(inferred)* Updates: download in the background and relaunch when idle, after every program is done | [str app] "Relaunch when Idle" | n/a |
| I13 | *(inferred)* Developer diagnostics: a network timeline and trace recorder (sizes and timings only, never message bodies), API endpoint inspector, debug info | [str app] | new |
| I14 | *(inferred)* Name this Mac for client lists and targeting (`-C`) | [str app] | have (C33) |

### J. Platforms and operations

| # | Capability | Evidence | rusty_mux |
|---|---|---|---|
| J1 | macOS app plus a bundled server; Linux and FreeBSD servers; Windows servers internal; an iOS client in alpha | [docs]; [cm] | part (C24 ahead on Windows) |
| J2 | Service units, socket activation, NixOS module, log following, JSON logs | [pkg] `lifecycle/logs.go`, `tail*.go`; [docs] | new (D11) |
| J3 | *(inferred)* utmp entries for terminals | [pkg] `internal/utmp` | new |
| J4 | No telemetry; Sparkle update checks only | [docs] privacy | par (C09) |

---

## 3. Adopted gaps

The owner adopted every row marked **new** or **part** that no capability covered (2026-11-02: "Add all of the gaps"). Rows marked *(inferred)* in §2 were adopted as directions, not as verified Rex behaviour. Each became a row in [CAPABILITIES.md](CAPABILITIES.md) and an entry in [ROADMAP.md](ROADMAP.md). Adopting a row places it; starting one still needs the owner's go, and a new crate, a new third-party dependency or a write outside the repository needs the owner's approval as usual.

| Row here | Capability | Phase |
|---|---|---|
| A7 | C46 Block ownership | 3 |
| F7, C7 | C47 Remote terminfo install | 3 |
| F6 | C48 SSH config setup, check and diagnosis | 3 |
| F8 (sync, home server, declared hosts) | C49 Host sync and a home server | 3 |
| A10, G5 | C50 Generated session names; agents name their session | 3b |
| E6 | C51 LLM-readable API help from the server | 3b |
| G1 (completion) | C52 Shell completion | 3b |
| C1 (`clear`, `modes`, `shell`) | C71 Block methods `terminal.clear`, `terminal.modes`, `terminal.shell` | 3b |
| C2 | C53 Kitty graphics through the mux | 4 |
| C3, C4 | C54 Clipboard and selection on the server | 4 |
| C9 | C55 Prompt marks and find in scrollback | 4 |
| E2 | C56 WebSocket API transport | 4 |
| F2 | C57 Pair a device by QR code | 4 |
| F9 | C58 Hub: several servers in one client | 4 |
| G3 | C59 Client settings overlay | 4 |
| G6 | C60 Config migration from other tools | 4 |
| I5–I8 | C61 Client themes, materials, fonts and shaders | 4 |
| I9 | C62 Pane focus and gestures | 4 |
| I10 | C63 Link safety | 4 |
| B5 | C64 Sessions across client windows | 4 |
| B4 | C65 Named layouts | 4 |
| C10 (theme kept through a reset) | C72 Theme kept through a reset | 4 |
| A3 | C45 Sessions survive a server restart | 5 |
| D1–D4 | C66 Block plugins and a block SDK | 5 |
| E5 | C67 API call history | 5 |
| I13 | C68 Developer diagnostics | 5 |
| J2, J3, A9 | C69 Service units, socket activation, utmp | 5 |
| C8 | C70 Scrollback spooled to disk | 5 |

Folded into existing rows: H2 (recording a shortcut) into C43, A8 (detached blocks) into C41, H3 (key routing) into C43, `blob_read` from C1 into C53, the keymap and Tailscale events from E7 into C43 and C27, F5 (anonymous listener) stays out by design, as SSH and login mode identify every peer. I12 (app updates) has no mux counterpart. The three open owner calls from the delta check stay open: FreeBSD as a server target, the scrollback budget (C70 depends on it), and version-skew errors with C36.
