---
category: Added
changelog: rusty-mux-ctl wait --window and wait --session wait until every block there has ended and exit with the last failure.
---
## 2026-11-01 - rusty_mux wait on a window or a session

- **Added:** `rusty-mux-ctl wait --window WINDOW` and `wait --session SESSION` (C37, Rex delta D5).
  - The wait ends once every block in the window or session has ended: its program exited or it closed. Blocks, and for a session windows, created during the wait join it.
  - Each exit is printed as an event line. The exit code is the last failure, as `set -o pipefail` reports: the program's status, or 1 with a message for a block closed before its program exited; 0 if every program succeeded. A window or session with no blocks ends at once with 0.
  - `--timeout` exits 124 and a missing or ambiguous target exits 3, as for `wait --block`. Nothing changes on the wire.
- **Known limitations:** blocks cannot move between windows yet; when layout editing (C41) adds that, a moved block must join or leave the wait.
