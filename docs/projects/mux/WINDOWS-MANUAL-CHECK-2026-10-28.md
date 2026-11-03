# rusty_mux Windows manual check, 2026-10-28

Mill Board card "Windows: two-account check and manual CUJs 1/3/4". Covers
R10, R11 and R13 in [RISKS.md](RISKS.md). The scripted checks were run by an
agent; the interactive ones were run by the owner at the keyboard and reported
in their own words. Nothing here is inferred: a check that was not run says so.

## Host

| Item | Value |
|---|---|
| Commit | `0bc18dd70` (`main`) |
| Windows build | 10.0.26300 (newer than 26100) |
| Shell for scripted checks | pwsh 7, **not elevated**, user `Beast\baileyrd` |
| Build | `cargo build --release -p rusty_mux_server -p rusty_mux_ctl -p rusty_mux_tui` |
| Socket | `%LOCALAPPDATA%\rusty-mux.sock` |
| Second socket (for CUJ 3, never used) | `C:\Users\Public\rusty-mux-r10.sock`, same single-entry ACL |
| Local accounts | `baileyrd`, `CodexSandbox*`; a `bob` account was created but CUJ 3 was not run |

`icacls` on the socket: one entry, `Beast\baileyrd:(F)`.

## Results

| # | Check | Expected | Observed | Result |
|---|---|---|---|---|
| 1 | Socket ACL | One entry, full control for the owner | `Beast\baileyrd:(F)` only | pass |
| 2 | CUJ 2: create-session, create-window, create-block, then `attach + acquire-lease + input "echo HI^42\r"`, then `terminal.format --history 0 --trim` | Screen contains `HI42`; Linux JSON shapes and exit codes | `echo HI^42` printed `HI42`; every reply `{"ok":true,...}`, exit 0 | pass |
| 3 | C30: `run --wait -- cmd.exe /c exit 3` | Exit 3 | `24 program-exited ... code=3 signal=-`, exit 3 | pass |
| 4 | C30: `wait --block <b> --timeout 0.5` on an idle block | Exit 124 | `rusty-mux-ctl: timed out`, exit 124 | pass |
| 5 | C31: `terminal.process --block <b>` | One line: the block's `cmd.exe` pid, two tabs | `10848<TAB><TAB>`; `tasklist` shows PID 10848 is `cmd.exe` | pass |
| 6 | C33: `rename-block --name agent`, then `list`, then `--block agent` | `list` shows it; the name works as a target | `list` shows `name="agent"`; `terminal.title --block agent` returned the title | pass |
| 7 | CUJ 4: `start /b ping -t localhost`, then `close-block` | `cmd.exe` gone in about 1 s, `ping.exe` in about 6 s | both gone in 0.08 s | pass |
| 8 | CUJ 4 / R11: `start ping -t localhost` (no `/b`), then `close-block` | `ping.exe` survives | `cmd.exe` gone in 0.07 s; `ping.exe` (pid 31172) still alive after 12 s, killed by hand | pass (R11 confirmed) |
| 9 | CUJ 1: `rusty-mux <socket>` opens a `cmd.exe` prompt in the profile directory | Prompt appears in the profile directory | "Yes" | pass |
| 10 | CUJ 1: `dir` | Works | "yes" | pass |
| 11 | CUJ 1: non-ASCII key `é` | Appears at the prompt | "typing é worked" (pasted) | pass |
| 12 | CUJ 1: `Ctrl-]` then `d` detaches and prints the block id | Detach | **Did not work.** The console showed `^]d`. `Ctrl-]` then `Ctrl-]` showed `^]^]` | **fail** |
| 13 | CUJ 1: reattach with `rusty-mux <socket> <id>` | Screen repaints | Reattached after closing the tab (not a clean detach); screen repainted and `dir` worked | pass (only after an unclean disconnect) |
| 14 | C12: `Ctrl-]` then `\|`, `-`, arrows, `z`, `x` | Layout commands work | Not run: they need the prefix, which failed in row 12 | not run |
| 15 | CUJ 3 / R10: second account runs `list` | Exit 4; note which layer refused it | Not run: owner chose to skip | not run |
| 16 | R13: elevated client, same user | Record whether it is refused or works | `list` from an elevated console succeeded and printed the sessions, windows, blocks and clients | pass (works, not refused) |

## Failure: the `Ctrl-]` prefix is not caught (row 12)

- **Setup:** Windows Terminal, US keyboard layout, 10.0.26300, freshly built
  `target\release\rusty-mux.exe` (the running process path was confirmed; no
  other `rusty-mux` is on PATH).
- **Keys:** hold Ctrl, tap `]`, release, then tap `d`.
- **Observed:** the shell echoed `^]d`. `Ctrl-]` twice echoed `^]^]`.
- **Expected:** `Ctrl-]` `d` detaches. `Ctrl-]` `Ctrl-]` types a single `^]`
  (`crates/apps/rusty_mux/crates/tui/src/keys.rs`).
- **What it shows:** `list` confirmed the TUI client was the one attached to the
  block. The byte reached `cmd.exe` as typed text, so the client forwarded it
  as input instead of treating it as the prefix.
- **Ruled out by reading code:** the parser in `keys.rs` handles `0x1d` and
  `0x1d 0x1d` correctly, and `feed` is called on every keyboard read. That
  leaves the bytes arriving differently than `0x1d`, or the client in a state
  where `feed` is not applied.
- **Cause:** not found. This is a guess, not a finding: the Windows console
  input path (`platform-windows/src/sys/console.rs`, `read_console_utf8`) may
  deliver Ctrl+`]` in a form other than the single byte `0x1d` under Windows
  Terminal. Needs a byte-level capture of what `read_chunk` returns for that
  key.
- **Effect:** detach and every layout command (C12) are unusable from
  Windows Terminal. #1200 dropped the bare `Ctrl-]` detach, so there is no
  other way out except closing the tab. Closing the tab disconnects the client;
  the block keeps running and can be reattached.

## Other observations

- `terminal.format` requires `--history N`. The manual-check script omitted it
  and got `missing parameter history`.
- The server wrote one `client <id>: An existing connection was forcibly closed
  by the remote host. (os error 10054)` line to stderr for 14 clients in
  total, across the two servers. The log does not say which clients they were,
  so they may be the TUI clients that were closed or killed and not the ctl runs.
  Not investigated.
- Windows build 10.0.26300 is newer than 26100, so the older-host caveat for
  CUJ 4 stays open: it was not checked on a build older than 26100.

## Not checked

- CUJ 3 / R10: whether a second account is stopped at `connect` by the socket
  DACL, or only by the SID check. Still open (design Q4). The second server at
  `C:\Users\Public` was prepared so that bob could reach the socket's parent
  directory; `C:\Users\baileyrd` itself is not traversable by other accounts, so
  a refusal there would not have answered Q4.
- C12 layout commands, blocked by row 12.
- A clean `Ctrl-]` `d` detach followed by reattach.
- Windows builds older than 26100.
