---
category: Fixed
changelog: rusty_conhost ends client programs still attached when the 2 s close grace runs out (#1259).
---
## 2026-10-31 - Own console host ends programs that outlive the close grace

- **Fixed:** when an own-console session closes, every client still attached at the 2 s grace deadline is now terminated with `STATUS_CONTROL_C_EXIT`, the code CTRL_CLOSE ends it with. Before, a program that ignored the closed console's errors kept running, and grandchildren were never killed. The host opens each client with `PROCESS_TERMINATE` at its CONNECT, so the pid can't have been reused, and lets go at its DISCONNECT. Clients that exit during the grace are untouched.
- **Added:** `platform_windows::condrv::ClientProcess` (`open`, `terminate`), a public API addition under design §11.2.1. No new dependency, feature or Win32 admission.
- **Known limitations:** a client the host can't open, such as a protected process, still only gets CTRL_CLOSE, and is logged. Ends after a host panic or engine error are unchanged: those paths close without killing.
