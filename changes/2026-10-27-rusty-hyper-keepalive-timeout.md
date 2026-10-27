---
category: Fixed
changelog: A `rusty_hyper` client connection closed by its keepalive now reports `Error::Timeout("keepalive")` instead of `Error::Aborted`.
---
## 2026-10-27 - rusty_hyper: a keepalive close is a timeout

- **Fixed:** when a keepalive ping went unanswered, the keepalive aborted the connection's driver, and `Connection::closed`/`close` reported `Error::Aborted` ("the connection task was cancelled"). A caller could not tell a dead peer from a cancelled task. The keepalive now marks the timeout before it aborts, and the connection reports `Error::Timeout("keepalive")`. A failed ping means the driver has failed, but it may still be flushing its GOAWAY to a peer that stopped reading. The keepalive now gives it one keepalive `timeout` to end with its own error, then aborts it, so teardown stays bounded.
- **Tests:** `tests/lifecycle.rs` checks for `Timeout("keepalive")` on the first `closed`, a repeated `closed`, and the final `close`. Both tests reported `Aborted` before the fix. Two unit tests in `src/client.rs` run over an in-memory transport: a failed driver reports its own `H2` error, and a failed driver whose writes stall is still ended by the keepalive.
