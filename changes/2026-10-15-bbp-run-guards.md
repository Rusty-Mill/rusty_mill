---
category: Added
changelog: **BBP run guards.** `bbp pairs` refuses runs that are not comparable (reads_per_turn off the manifest, a role on two models, a different model per role across arms) or not pinned (the binary's build commit must equal the clean checkout's HEAD), `bbp mod --max-usd C --criteria FILE` stops a run live when its harnesses cost more than C, and the decision table has a test for every row and for first-match-wins.
---
## 2026-10-15 - BBP run guards

- **Added:** `bbp pairs` manifest field `reads_per_turn` (every run's card must match it) and checks that each role used exactly one model per run, the same one in both arms, and that an extra N=2 role used a model the control also used; a violation refuses the evaluation. The models come from the harness output, so this is observed, not declared; pinning the model is still done with `--model` in each role's launcher in the agents file.
- **Added:** `build.rs` records the build commit (`-dirty` or `unknown` when not clean) as `rusty_bbp_host::BUILD_SHA`; `bbp mod` prints it as `bbp_build_sha` and `bbp pairs` refuses unless it equals the checkout's HEAD, which must be clean and contain `min_commit`. The output records both.
- **Added:** `bbp mod --max-usd C --criteria FILE` checks the dollar cost of every turn's harness output each poll and, over C, kills the harnesses and fails the run (a run that does not close is a failed run). It also fails closed when a model has no price. It lags by a poll and by what a harness has printed, so it cuts a run short and does not bound spend exactly; the API key's spend limit remains the hard stop.
- **Tests:** a table-driven test for every decision row alone and for first-match-wins (each row against every later one; mutating the order of two rows fails three tests), the settings checks, the pin with the build commit, and the cap over, under and with an unpriced model.
- Host only: no engine, schema or log path.
