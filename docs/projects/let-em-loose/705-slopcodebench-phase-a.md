# #705 Phase A: rusty_erosion against SlopCodeBench (2026-11-26)

Result: **both pre-registered rules pass.** rusty_erosion's concentration and duplication track SlopCodeBench's own scores on all 196 reference-solution checkpoints. No model was called.

## What was run
- **Snapshots:** every reference solution in `gabeorlanski/scb-problems` at `9cd9ca3a` (Apache-2.0): 36 problems, 196 checkpoints, 160 checkpoint-to-checkpoint steps (O1 a, D201).
- **Ours:** `rusty_erosion` with the Python adapter (PR #1907), through `examples/measure_tree.rs`.
- **Theirs:** `scb-check` 0.1.3, the version the SCB harness pins, run with `check --report --include-all` (Nano approved running it here, 2026-11-26 14:15Z).
- **Reproduce:** `crates/tools/rusty_erosion/scb/score.sh <scb-problems> > rows.tsv`, then `python3 crates/tools/rusty_erosion/scb/summarize.py report rows.tsv`. It takes about 3 minutes. The rows are committed as [705-slopcodebench-rows-2026-11-26.tsv](705-slopcodebench-rows-2026-11-26.tsv), and a test checks that they still produce the table below.

## Result
Snapshots: 196 in 36 problems; steps: 160.

| Pair | Spearman rho | Direction agrees | Rule | Verdict |
|---|---|---|---|---|
| concentration vs SCB erosion | 0.949 | 123/145 (85%) | rho >= 0.6 and >= 75% | agree |
| duplication vs SCB clone share | 0.636 | 94/148 | rho >= 0.5 | agree |
| duplication vs SCB verbosity | 0.493 | | none | reported only |

Worst concentration/erosion rank disagreements (gaming check: functions, total CC):

| Problem | Checkpoint | concentration | SCB erosion | functions | total CC |
|---|---|---|---|---|---|
| cfgpipe | 2 | 0.750 | 0.668 | 7 | 68 |
| dynamic_config_service_api | 3 | 0.340 | 0.745 | 154 | 683 |
| mvvault | 6 | 0.214 | 0.579 | 114 | 477 |
| cfgpipe | 3 | 0.547 | 0.528 | 13 | 95 |
| dynamic_config_service_api | 4 | 0.363 | 0.753 | 243 | 1112 |

## Reading it
- **Concentration and SCB erosion agree strongly** (ρ 0.95; the direction of change agrees on 85% of steps). Part of this is by construction: the Python adapter counts CC by radon's table, which is the CC SCB uses. What the test adds is that the two formulas rank the same snapshots the same way, even though SCB weights each function by CC·√SLOC and rusty_erosion doesn't weight at all, and the two use different parsers.
- **Duplication and SCB's clone share agree moderately** (ρ 0.64, above the 0.5 bar). The two detectors differ: ours uses normalized 50-token windows, theirs counts clone lines. The direction agrees on only 94 of 148 steps (64%). No rule was set for that, so it's reported without a verdict.
- **Duplication against verbosity** is weak (ρ 0.49), as expected. Verbosity is mostly SCB's 137 AST-grep rules, and rusty_erosion has nothing that measures that kind of style.
- **The largest disagreements are big API-style files with many functions** (`dynamic_config_service_api`, `mvvault`). The likely cause, not yet checked function by function: a few long functions carry a large share of SCB's √SLOC-weighted mass, while rusty_erosion's unweighted CC share stays low. This is the D45 gaming shape in reverse: rusty_erosion reads lower when complexity is spread over many medium-sized functions.

## Limits
- **Python only.** This supports the shared core and the Python adapter. It doesn't directly show that the Rust adapter is right (design §3 A.5).
- **The snapshots were written by the benchmark's authors, not by agents.** Agent code erodes faster (the paper's agent erosion is 0.68, against 0.34 for human repositories), so Phase B's agent trajectories are the harder test.
- **One scorer version.** scb-check rules change between releases, so the result holds for 0.1.3.

## Next
Phase B, the factory against a control on 3 problems, waits on #1320 and its own paid approval (D134). That ask will restate O2 and O3 (D201).
