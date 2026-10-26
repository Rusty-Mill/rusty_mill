---
category: Added
changelog: rusty-mux-server runs on Windows, hosting each block on a ConPTY pseudo console behind an AF_UNIX socket that only the current user can reach.
---
## 2026-10-26 - rusty-mux-server on Windows

- **Added:** the server builds and serves on Windows 10 1803 or later. Each block runs `%COMSPEC%` (else `cmd.exe`) in `%USERPROFILE%` on a pseudo console, with a reader thread that always drains its output and a writer thread for input, so a slow or flooding block never blocks the server's thread. Closing a block hangs up its console (every attached program gets the console-close notification) and terminates the program after one second.
- **Added:** access control on Windows, as on Linux in two layers: the socket file gets an owner-only DACL after `bind`, and each connection's peer process must run as the server's user (token user SID), else it is refused and logged by pid.
- **Changed:** the server's PTY module is split into a shared part and Linux and Windows backends with one interface; the peer check is passed to the accept loop as a function. Linux behaviour is unchanged.
- **Known limitations:** ConPTY re-renders output, so OSC 7501 status and other passthrough sequences may not survive on Windows. `terminal.pwd` falls back to OSC 7 only. Detached descendants of a block's program can outlive it (no Job Object yet). The events, inspect and lifecycle integration tests stay Linux-only, and refusing a second interactive user is a manual check. Windows tests run only on CI's Windows leg.
