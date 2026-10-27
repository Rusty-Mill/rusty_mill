---
category: Added
changelog: The rusty-mux attach client runs on Windows; Windows console input no longer reports key releases as input and arrives as UTF-8 whatever the code page.
---
## 2026-10-27 - rusty-mux attach client on Windows

- **Added:** `rusty-mux` attaches to a Windows `rusty-mux-server` over AF_UNIX, using `platform_windows::WindowsTerminal` for raw mode and keyboard input.
- **Added (rustils 0.29.0):** `WindowsUnixStream::try_clone` and `shutdown`, so the client reads on one thread, writes on another and ends the read on detach.
- **Changed (rustils 0.29.0):** on a Windows console, `poll_readable` drops queued input records that carry no text (key-ups, bare modifiers, focus and mouse records) and keeps waiting, so a key release no longer wakes the client's input loop into a blocking read. The key that types NUL (Ctrl+@) still counts. Piped stdin keeps the plain wait.
- **Fixed (rustils 0.29.0):** on a Windows console, `read_chunk` returns UTF-8 whatever the input code page, so non-ASCII keys reach a block's ConPTY intact.
- **Known limitations:** the client's lifecycle tests (`tui/tests/lifecycle.rs`) drive it on a Linux pty and stay Linux-only, so on Windows the client is only built and linted. The new rustils tests run only on CI's Windows leg, and the client was not run interactively on Windows.
