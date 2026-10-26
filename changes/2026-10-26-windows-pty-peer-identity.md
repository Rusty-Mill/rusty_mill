---
category: Added
changelog: platform-windows 0.28.0 gains the Windows pieces rusty-mux-server needs (peer pid, same-user check, owner-only socket file, ConPTY hang-up and a resize that never reaches a closed console); rusty_tokio gains UnixStream::peer_pid on Windows.
---
## 2026-10-26 - Windows peer identity and ConPTY hang-up for rusty_mux

- **Added:** `WindowsUnixStream::peer_pid` and `rusty_tokio::io::UnixStream::peer_pid` (Windows) report the peer process from `SIO_AF_UNIX_GETPEERPID`; `platform_windows::is_current_user` compares its token user with ours; `platform_windows::fs::restrict_to_owner` gives a socket file an owner-only DACL. Together they stand in for `SO_PEERCRED` and `chmod 0600` on Windows.
- **Added:** `WindowsPty::spawn_concrete` and `WindowsPtyMaster::hang_up`; `WindowsUnixStream` implements `std::io::Read`/`Write` and has `set_read_timeout` (std's contract: zero is refused, sub-millisecond rounds up). The owner-only DACL does not isolate the file from administrators: its owner keeps implicit `WRITE_DAC`.
- **Changed:** the ConPTY close guard is now a lock (`close_guard::CloseGuard`, unit-tested on every host with injected blocking) held across every close and every resize. A resize after the console closed fails with `BrokenPipe` instead of using a closed handle; dropping the master never waits on a close in progress. Console creation is serialized process-wide.
- **Known limitations:** the live Windows tests run only on CI's Windows leg (merge queue). Refusing a different interactive user needs a second account and stays a manual check. A peer pid can name another process if the peer has already exited. The PAL group version moves to 0.28.0.
