---
category: Added
changelog: The lens runner reads a claim's current code candidate and its diffs for the alien lens (#708).
---
## 2026-11-05 - Alien lens candidate reads

- **Added:** `orch_lens::card::Card` carries `current`, the claim's current code candidate when the lens may read it.
- **Added:** `Session::candidate_diffs` reads that candidate and then each of its diffs with `get_artifact`, and returns their text in order. Each artifact's first read is charged once to the lens principal under R11, not to the claim's budget. If the lens's role may not read the candidate or a diff, the answer is `Diffs::Forbidden`, which costs nothing and makes the claim ineligible (D87). Any other failure is an error.
- **Added:** `orch_lens::alien::touches_rs` decides from diff headers (`--- a/`, `+++ b/`, `rename from`, `rename to`, `diff --git`) whether a candidate touches a `.rs` file. `MAX_DIFF` (128 KiB) is the largest diff text a hunt runs on. A larger one is skipped, never truncated.
- **Known limitations:** nothing calls these yet. The `--alien` runner pass uses them in the next slice.
