---
category: Fixed
changelog: rusty_conhost ends client programs still attached when the console closes, 5 s after CTRL_CLOSE (#1259).
---
## 2026-10-31 - Own console host ends programs that outlive the close grace

- **Fixed:** when an own-console session closes, every client still attached at the 2 s grace deadline is now terminated 5 s after CTRL_CLOSE, Windows' own bound on a CTRL_CLOSE handler, with `STATUS_CONTROL_C_EXIT`, the code CTRL_CLOSE ends it with. The console itself still closes at 2 s. Before, a program that ignored the closed console's errors kept running, and grandchildren were never killed. The host opens each client with `PROCESS_TERMINATE | SYNCHRONIZE` at its CONNECT, so the pid can't have been reused, and lets go at its DISCONNECT. Clients that exit during the grace are untouched.
- **Added:** `platform_windows::condrv::ClientProcess` (`open`, `terminate`), a public API addition under design §11.2.1. No new dependency, feature or Win32 admission.
- **Known limitations:** a client the host can't open, such as a protected process, still only gets CTRL_CLOSE, and is logged. After a host panic or engine error the console closes without ending clients, as before, and if the host process itself is killed nothing ends them.
