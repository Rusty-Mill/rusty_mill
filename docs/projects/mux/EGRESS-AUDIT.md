# C09 egress audit

Card #266, capability C09: self-hosted, with no account and no telemetry. This is a baseline run on 2026-11-12, on main at the commit this PR branches from. ROADMAP's Phase 5 exit repeats it on the release build.

**Result:** none of the mux binaries opens a network connection the user didn't ask for. No call site is unexplained, and there is nothing to fix.

## Dynamic: `.github/scripts/mux_egress_audit.sh`

The script runs the release `rusty-mux-server`, `rusty-mux-ctl`, `rusty-mux-mcp` and `rusty-mux` (TUI) through a scripted session, each under `strace -f -e trace=socket,connect,sendto,sendmsg,bind`. The session:

- creates a session, a window and a block running `/bin/echo`;
- runs `list` and `whoami`;
- runs an MCP `initialize` and `tools/list`;
- attaches the TUI through a pty for 3 s.

The script fails if any traced process opens an `AF_INET`, `AF_INET6`, `AF_PACKET` or `AF_NETLINK` socket, or connects, sends or binds to an IP address. It also fails if a step did not run: mcp must answer `tools/list`, and the TUI must leave a trace.

On Linux, all 27 traced calls were `AF_UNIX`, and the script printed `egress audit: PASS`.

Negative check: `rusty-mux-server --udp 127.0.0.1:0` produces `socket(AF_INET, SOCK_DGRAM…)` and an `AF_INET` `bind`, and the script's filter catches both.

The GUI (`rusty-mux-gui`, `rusty_term --gui`) needs a display, so it is covered by the static audit below only.

## Static: every network call site

The search covered `UdpSocket`, `TcpStream`, `TcpListener`, `connect`, `send_to`, `to_socket_addrs`, `lookup_host`, URLs and `Command::new`, outside tests. It ran over every crate under `crates/apps/rusty_mux/crates/`, plus `rusty_term` and `rusty_conhost`. No dependency of the server, ctl, TUI, GUI or mcp is a network or HTTP client (`cargo tree`).

| Site | What it does | When |
|---|---|---|
| `server/src/main.rs:269`, `endpoint.rs` | Binds the Unix socket and probes it for a live server | Always; local only |
| `server/src/main.rs:277`, `datagram.rs` | UDP listener and its replies | Only with `--udp ADDR` |
| `server/src/login.rs` | Unix socket to per-user servers; runs the per-user server binary | Only in `--login` mode |
| `tui/src/udp.rs`, `tui/src/main.rs:776` | Resolves the server host (DNS), then a UDP association | Only when the server offers UDP, which needs `--udp`, and a remote attach the user started |
| `client/src/ssh.rs`, `client/src/ssh/forward.rs` | Runs `ssh` | Only with `--ssh` or `--host` naming an ssh address |
| `ctl/src/ssh.rs`, `ctl/src/sync.rs`, `ctl terminfo` | Runs `ssh`, `ssh -G`, `ssh-keygen` | Only for the `ssh`, `hosts pull/push` and `terminfo` verbs, on a DEST the user names |
| `client/src/autostart.rs` | Starts a local `rusty-mux-server` | When nothing answers on the default socket |
| `rusty_term/src/gui/window.rs:6448` | `xdg-open`, `open` or `cmd start` on a link | Only on Ctrl+click of an openable link (`is_openable_url`) |
| `rusty_term/src/gui/window.rs:6515-6534` | Desktop notification (`notify-send`, `osascript`, PowerShell) | Program-requested notifications; local only |
| `rusty_term/src/config.rs:423-446` | Opens the config in an editor | User action |
| `rusty_term/src/web_bridge/` | WebSocket listener, default `127.0.0.1:7703`; runs `git` for stats | Only the separate `web_bridge` binary; loopback unless `--listen` says otherwise |

There are no update checks, crash reporting, analytics, licence calls, or font or theme downloads.

## Keeping it

The script is cheap: about 5 s after a release build, and it needs `strace` and `script`. It isn't wired into CI yet. It can run as a Linux-only job, or before each release as part of the Phase 5 exit.
