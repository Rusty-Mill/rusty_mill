---
category: Fixed
changelog: Preserve task cancellation when abort races the queued-to-running transition in rusty_tokio.
---
## 2026-10-24 - Task abort and poll transition race

- **Fixed:** Retry the queued-to-running transition when a concurrent abort changes its state bits, preserving cancellation instead of panicking outside the future-poll panic handler and abandoning join completion. Finish the terminal state transition when ordinary or local tasks are cancelled before a poll.
- **Validation:** Deterministic load/abort/CAS regression, cancellation state and join-waker checks, gated abort-during-poll tests, and bounded abort/join stress with all result kinds counted and one future drop per task.
- **Known limitations:** This fixes a confirmed pre-existing interleaving; the cause of the separately reported one-off abort/await hang has not been established.
