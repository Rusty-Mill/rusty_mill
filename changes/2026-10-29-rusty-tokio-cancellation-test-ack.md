---
category: Fixed
changelog: A rusty_tokio cancellation test no longer races its consumer; it waits for the consumer's receipt of every message before cancelling.
---
## 2026-10-29 - rusty_tokio: cancellation select-loop test waits for receipt

- **Fixed:** `select_loop_on_cancelled_exits_once_cancelled` slept 20 ms after sending 50 messages, then cancelled and expected all 50 received. A slow runner could start the consumer after the cancel, and `select!` may then pick the ready `cancelled()` arm first. This failed once on Windows in the merge queue for #1221. The consumer now reports receipt of all 50 through a oneshot, and the test cancels only after that, within a bounded timeout. The exact count assertion is unchanged.
