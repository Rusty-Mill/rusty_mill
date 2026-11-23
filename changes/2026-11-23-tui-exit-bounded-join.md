---
category: Fixed
changelog: The rusty-mux client no longer hangs on exit when the server keeps its connection open (Windows).
---
## 2026-11-23 - rusty-mux exit waits a bounded time for its readers (#1451)

- **Fixed:** on Windows, shutting a socket does not wake a blocked read, so the client's exit joined a reader that never returned if a server kept the connection open. Exit now waits up to 2 seconds in total for all its readers (one shared budget) and leaves the rest to the process end; dropping a connection in a running client waits at most 250 ms for its reader.
- **Added:** a layout-server mode that keeps the control and both block connections open after the client leaves, and a test that detaching returns within a deadline against it.
- **Known limitations:** the Windows run of the new test is left to CI; it was run on Linux only.
