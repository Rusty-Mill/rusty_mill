---
category: Fixed
changelog: **BBP host e2e race.** The two-builder e2e asserted the moderator's total launched turns, which also counts a tester turn granted once the runner reports; it now counts the builders' turns from the log.
---
## 2026-10-14 - BBP host e2e race

- **Fixed:** `two_builders_run_in_parallel_and_the_candidate_kills_the_other_harness` asserted `turns_launched == 3`. After the candidate the runner reports and the tester is granted a turn (no launcher, still counted), so the count was 4 whenever the cancel landed late. It failed once in a full-suite run. The test now asserts at least 3 and exactly one turn each for planner, coder and coder2, read from the log. Reproduced by delaying the cancel by 1.5 s (count 4) before the change.
- **Known limitations:** test only; no product change.
