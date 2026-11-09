# C69: server lifecycle

C69 ([CAPABILITIES](CAPABILITIES.md) row; [Rex app check](REX-APP.md) A9, J2, J3; Rex delta D11)
makes the server something a user doesn't have to start by hand: a default socket, auto-start
and idle exit, systemd units with socket activation, and utmp entries. Nano gave the go on
2026-11-06 23:15Z. The plan and its choices are the rusty_term handoff pack's D130-D135
(`docs/projects/rusty_term/handoff/DECISIONS.md`).

Retiring rusty_term's GUI local mode needs slices (a) and (b) plus C45. It is a separate step,
not part of C69, and it needs Nano's go ([term ROADMAP](../term/ROADMAP.md) question 1).

## Slices

| Slice | What | State |
|---|---|---|
| (a) | Own the socket's name (Orca O6) and a default socket, so the socket argument is optional | done (#1555) |
| (b) | Auto-start from `rusty-mux` and `rusty-mux-gui`, and `--idle-exit SECS` | done (#1558) |
| (c) | systemd user units, socket activation (`LISTEN_FDS`) | this PR |
| (d) | utmp entries | deferred: needs setgid `utmp` or `libutempter`, both owner gates (pack Q29) |

FreeBSD is not a server target (pack D130, Nano "Q8 not now").

## (a) The socket's name

**Default socket.** A socket is the one a command names, else `$RUSTY_MUX_SOCKET`, else the
default: `$XDG_RUNTIME_DIR/rusty_mux/default.sock` on Linux,
`%LOCALAPPDATA%\rusty_mux\default.sock` on Windows (`rusty_mux_proto::endpoint`). With the
variable unset there is no default and the error says what to set; there is no `/tmp`
fallback (pack Q27). The server, `rusty-mux`, `rusty-mux-gui`, `rusty-mux-ctl` and
`rusty-mux-mcp` all follow this; `rusty-mux --ssh` still needs a socket, and ctl's agent hook
reports never use the default (outside a block they do nothing). Every invocation that named a
socket before behaves as it did.

When the server's socket is the default, it makes the directory itself, 0700, and checks it as
login mode checks its runtime directory: each part a real directory, root's or the user's,
writable by no one else; the last one the user's alone. A directory others can open is refused.
The C45 state directory follows the socket's file name, so the default server's is
`$XDG_STATE_HOME/rusty_mux/default.sock`.

**Owning the name (O6).** A server never removes a name it didn't make
(`rusty_mux_server::endpoint`, Linux):

1. Bind a temporary name, `.rusty-mux.<pid>`, beside the target, under a umask that gives the
   socket its mode as it is made (0600; 0666 for login mode's front door), so no chmod follows.
2. Hard-link it to the target. That fails if anything is there; if it succeeds the name is ours.
3. If something is there and it is not a socket, refuse and leave it.
4. If a server answers there, refuse (`a server already answers at ...`, exit 1).
5. If it is a socket nothing answers on, rename the temporary name over it.
6. Remove the temporary name in every case.

On a clean stop the server removes the name only if it is still the socket it bound (same
device and inode), so it never takes away a successor's. Login mode's front door uses the same
helper; before, it removed a stale socket and then bound.

On Linux a signal still ends the server without a clean stop, so the name stays and the next
server replaces it at step 5. On Windows, `bind` already replaces only a socket nothing answers
on (`platform-windows` `unix_listen`), and the server leaves the name on stop for the same
reason.

Two servers racing for a stale name can both reach step 5; the later rename wins and the earlier
server serves a name no one can reach until it exits. With `--idle-exit` (slice (b)) such a
server ends on its own, since no client can reach it.

## (b) Auto-start and idle exit

**Idle exit.** `rusty-mux-server --idle-exit SECS` (personal server only) stops cleanly once it
has had no block, no client and no orphaned client a takeover could still claim for SECS
without a break (`State::is_idle`, looked at every 250 ms). It never stops while a block lives,
so no program is ended and nothing needs restoring. The stop is the clean one: state is saved
(C45) and the socket's name is removed while it is still the server's. A server started by hand
or by systemd keeps running unless given the flag (pack D133).

**Auto-start** (`rusty_mux_client::autostart`). Only `rusty-mux` and `rusty-mux-gui` start a
server, and only when the socket is the default one: no socket named and no
`$RUSTY_MUX_SOCKET`. ctl, MCP and `--ssh` never do. If nothing answers there, the client runs
`rusty-mux-server --idle-exit 60 <socket>`. It uses the copy beside its own program if there is
one, else the one on `PATH`. The server is detached: its own process group on Unix, no window
on Windows, no stdio. The client then tries to connect every 25 ms for up to 5 s.

Its own process group is enough to outlive the client's terminal (pack D142's check): a hangup
sends SIGHUP to the terminal's foreground group, which the server is not in, so the spawn needs no
`setsid`. Checked by hand on 2026-11-09 under `script`: closing the terminal killed a background
process left in the shell's group, while one in its own group, as the server is, kept running.

If two clients each start a server, the servers' own bind settles it: the second refuses the
first's live name and exits, and both clients connect to the first. A started server that exits
while nothing answers is an error that tells the user to run the server by hand to see why.

A started server is the client's child until the client exits. If it idle-exits first, it
stays a zombie until then. The client has already disconnected by that point, so this is
short-lived.

## (c) systemd

User units ship in the repo, at `crates/apps/rusty_mux/crates/server/dist/systemd/`, and are
installed by hand (instructions in the files). There is no installer command, because that would
write outside the repo (pack Q28), and there is no Windows service (pack D134).

- `rusty-mux.socket` listens on `%t/rusty_mux/default.sock`, which is the default socket. Its
  settings are `SocketMode=0600`, `DirectoryMode=0700` and `RemoveOnStop=yes`.
- `rusty-mux.service` runs `rusty-mux-server` and gets the listener from systemd.
- The server takes the listener (`rusty_mux_server::endpoint::inherited`) only when `LISTEN_PID`
  is its own pid and `LISTEN_FDS` is 1. It refuses more than one socket, and refuses an fd 3 that
  is not an open Unix socket. It makes the listener close-on-exec, which systemd leaves clear, so
  no block's program inherits it.
- The server removes the `LISTEN_*` variables before anything starts, so blocks' programs don't
  inherit them.
- When it has the listener from systemd, the server binds nothing and never removes the name.
  `--idle-exit` still works: the server stops, systemd keeps the socket, and the next connection
  starts the server again.

Checked with `systemd-socket-activate` and covered by `server/tests/endpoint.rs`.

The Windows test on Beast (2026-11-07, main b8ae0b9b) passed. `rusty-mux-gui` with no arguments
started a server with no window, a second GUI shared it, and the server outlived both GUIs while
its blocks lived. After the blocks closed, it stopped about 60 s later; when the GUI had been
killed rather than closed, it took about 30 s longer (`ORPHAN_GRACE`).

C69 is done except (d), utmp entries, which is deferred (pack Q29).
