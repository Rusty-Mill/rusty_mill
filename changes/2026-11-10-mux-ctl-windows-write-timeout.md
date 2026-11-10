---
category: Fixed
changelog: rusty-mux-ctl on Windows now bounds a write to a server that stopped reading by the command's deadline, so an agent hook can no longer hang on it.
---
## 2026-11-10 - rusty-mux-ctl write timeout on Windows

- **Fixed:** on Windows, `rusty-mux-ctl` writes had no timeout, so a command with a deadline (agent hooks, `wait --timeout`, MCP calls) could block for ever on a server that stopped reading. Writes now time out at the command's deadline and end the command as a timeout (card #282, item 2).
- **Added:** `platform_windows::WindowsUnixStream::set_write_timeout` (`SO_SNDTIMEO`), with the contract of std's `UnixStream::set_write_timeout`.
- **Known limitations:** Winsock leaves a connection whose send timed out unusable, so on Windows a write times out once, at the deadline, instead of polling every 50 ms as on Unix; a stop with only a cancel check and no deadline still writes without a bound. Connecting is still unbounded on Windows.
