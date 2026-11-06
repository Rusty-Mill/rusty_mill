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
| (a) | Own the socket's name (Orca O6) and a default socket, so the socket argument is optional | this PR |
| (b) | Auto-start from `rusty-mux` and `rusty-mux-gui`, and `--idle-exit SECS` | next |
| (c) | systemd user units, socket activation (`LISTEN_FDS`) | after (b) |
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
server serves a name no one can reach until it exits. Slice (b)'s idle exit ends such a server.

## (b) Auto-start and idle exit (next)

Only `rusty-mux` and `rusty-mux-gui` auto-start, and only for the default socket (no argument,
no `$RUSTY_MUX_SOCKET`): they start `rusty-mux-server --idle-exit 60` detached, then retry the
connect for a few seconds. ctl, MCP and `--ssh` never do. `--idle-exit SECS` exits once the
server has had no clients and no blocks for SECS, never while a block lives. A server started
by hand or by systemd keeps running unless given the flag (pack D133).

## (c) systemd (after (b))

User units `rusty-mux.service` and `rusty-mux.socket` ship in the repo under
`crates/apps/rusty_mux/crates/server/dist/systemd/`, installed by hand; the server takes the
listener from `LISTEN_FDS`/`LISTEN_PID` and never removes the name systemd owns. No installer
command (a write outside the repo, pack Q28) and no Windows service (pack D134).
