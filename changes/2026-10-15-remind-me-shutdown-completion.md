---
category: Fixed
changelog: Wait for remind_me daemon sessions and background workers to release the store before reporting shutdown success.
---
## 2026-10-15 - Confirm daemon shutdown completion

- **Fixed:** `daemon stop` now waits for connection draining and background cleanup before reporting `stopped`, so an immediate local command can reopen the engine store. Shutdown requests have a 30-second deadline and report timeout, disconnect and cleanup failure instead of success.
- **Known limitations:** An older daemon's early acknowledgment cannot prove completion; the new client requests shutdown but returns an explicit unconfirmed-completion error. Retry stop after that older process exits. A stalled worker is not forcibly killed.
