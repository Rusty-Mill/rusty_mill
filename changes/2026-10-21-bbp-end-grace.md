---
category: Fixed
changelog: **BBP moderator: an ended turn's harness may finish.** `bbp mod --end-grace-secs` (default 60) lets it print its `result` line before it is killed, so evidence runs can price their turns.
---
## 2026-10-21 - BBP moderator: grace for an ended turn's harness

- **Fixed:** `bbp mod` killed a harness on the first poll after its turn ended. A real harness prints its `result` line only after its terminal call is answered and the model replies once more, so almost every turn's cost read as unknown and, under the approved 2b criteria, every evidence run would have failed. A harness whose turn ended (its own terminal call or a revocation) now gets `--end-grace-secs` (default 60) to exit on its own, also when the task closes; then it is killed. Cap and wall stops still kill at once.
- **Docs:** the design note's cost-cap section says so, and that a revoked N=2 author's prompt must stop on `stale_turn` or its pair fails.
- **Known limitations:** spend during the grace is not checked by `--max-usd` after the task closes; it is bounded by the grace and still counted from the `result`. No criterion, threshold or price changed.
