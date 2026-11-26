# #705 SlopCodeBench spike: design (draft 2026-11-26)

Status: proposal for Nano. Nothing is built, no model has been called, and nothing here authorises a paid call (D134). Build nothing until Nano approves on card #705.

## 1. Goal
Two questions, report only (card #705):
1. **Does the factory slow decay?** Run SlopCodeBench (SCB) checkpoints through the real pipeline and compare erosion and verbosity curves.
2. **Is `rusty_erosion` trustworthy?** Score the same snapshots with SCB's metrics and with `rusty_erosion`, and check that they move together.

Out of scope: changing the #704 gate (card).

## 2. What SCB is (read 2026-11-26)
- Paper: arXiv 2603.24755 v2. Harness: `SprocketLab/slop-code-bench` (MIT, Python 3.12, uv, Docker). Problems: `gabeorlanski/scb-problems` (Apache-2.0).
- 36 Python problems with 196 checkpoints, 3 to 8 per problem. Each checkpoint is a spec extension. The agent gets the new spec plus its previous workspace, in a fresh container with no earlier conversation. Tests are hidden: core, error, functionality and regression, all pytest.
- **Every checkpoint has a reference solution** in `scb-problems/<problem>/solutions/checkpoint_N/`. That gives 196 snapshots that need no model to produce.
- **SCB erosion** = Σ over callables with CC > 10 of CC·√SLOC, divided by the same sum over all callables (radon CC). **SCB verbosity** = (lines flagged by 137 AST-grep rules ∪ clone lines) / LOC. The harness scores any directory offline (`slop_code.metrics.measure_snapshot_quality`).
- **`rusty_erosion` is close but not the same.** Concentration is the unweighted share of CC in functions above CC 10. Duplication is the share of tokens in normalized windows of 50 or more tokens. So agreement can only be tested by rank, not by equal values.
- Paper baselines, per model across all 196 checkpoints (examples): Opus 4.6 has erosion 0.75, verbosity 0.44, strict solve 9.7%, $3.17 per checkpoint. GPT 5.5 has 0.49, 0.32, 14.8%, $1.51. Erosion rises in 77% of trajectories. Human repos average erosion 0.34. Anti-slop prompts lower the level, not the slope. **No per-problem or per-checkpoint baseline data is published**, so a subset can't be compared with the paper like for like.

## 3. Plan: two phases

### Phase A: cross-validation ($0, no models; starts after this design is approved, before #1320)
1. **Python adapter for `rusty_erosion`:** add `adapter/python.rs` (D52 anticipates it; D45 allows a language "when a real task needs it"). It mirrors the Rust adapter: CC per function and normalized tokens. It skips `tests/`, `test_*.py` and `*_test.py`. Every other language stays `unmeasured`. The core does not change.
   - **Dependency ask:** this adds `tree-sitter-python` 0.25 to `rusty_erosion`. The version is already pinned in the workspace `Cargo.toml`, but the crate is not in the root `Cargo.lock` yet, so the lock gains an entry.
2. **Snapshots:** all 196 SCB reference-solution checkpoints, as 36 trajectories.
   - Optional addition: commit histories of about 30 public Python repos, with licences checked, using the paper's human-panel method. This widens the sample beyond benchmark-author code. It goes beyond the card's "same checkpoints", so Nano chooses it.
3. **Scoring:** run SCB's metric code and `erosion` on every snapshot.
   - Report function count and total CC next to concentration (D45, D110), so that splitting functions doesn't look like improvement.
4. **Correlation (pre-registered, proposed):**
   - **Levels:** Spearman ρ between `rusty_erosion` concentration and SCB erosion over all snapshots. Also ρ between `rusty_erosion` duplication and SCB's clone-line share, and between duplication and verbosity.
   - **Deltas:** for each checkpoint step, does `rusty_erosion` move the same way as SCB? Steps where both show zero change are excluded.
   - **"Agree"** means ρ ≥ 0.6 on levels **and** at least 75% sign agreement on deltas, for concentration against erosion. Duplication against clone share uses ρ ≥ 0.5. Anything lower is reported as "disagree", with the worst disagreements listed.
5. **Limit to state:** agreement on Python supports the shared core and the Python adapter. It does **not** directly show that the Rust adapter is right; the card's "trustworthy on Rust" is inferred, not measured.
6. **Where it runs and what it produces:** a cloud session with no provider calls (the D110 precedent). The harness stays outside the repo, in scratch, and is not vendored. Output is a committed TSV with one row per snapshot, plus `705-slopcodebench-report.md`.
7. **Bonus:** the reference trajectories also give a "reference author" erosion curve per problem. Phase B can be compared against it.

### Phase B: factory run (paid; waits on #1320 and a separate approval)
- **Subset:** 3 short problems with 11 checkpoints: `dag_execution` (3), `file_backup` (4) and `dynamic_buffer` (4). Nano may choose others.
- **Factory arm:** each checkpoint is one claims task.
  - The intent is `checkpoint_N.md` verbatim. The base is the previous checkpoint's accepted workspace. Builders are Claude CLI only (D64, D188: candidates only with `.lel/submit` and a non-empty diff). The lenses are the normal ones (STARTER), including the holdout lens writing its own holdouts from the intent (D37). The erosion profile is report-only (D42, D115: there are no Python limits).
  - **SCB tests never enter BBP.** They are not profiles, holdouts or golden sets. They score the accepted workspace only after the task ends (D37 holdouts would gate acceptance and leak pass counts).
  - If no claim is accepted, the workspace carries forward unchanged and the checkpoint scores as it stands.
  - D44 scheduled cleanups are **off** by default, so the run matches the paper's setup; see option O3.
- **Control arm (proposal, needs Nano):** one bare Claude CLI builder session per checkpoint, with the same model, spec and base and no lenses. The paper has no per-problem baselines, so without this arm the comparison is only against aggregates (the D57/#596 baseline-versus-treatment pattern).
- **Billing path:** both arms use the subscription CLI with the `no_overage` assertion, through orch's admission. SCB's own API-key harness is **not** used (ADR-0021, D38, D167). SCB is used only to score the snapshots.
- **Scoring:** strict, ISO and core pass rates, SCB erosion and verbosity, and `rusty_erosion` for every checkpoint in both arms. Erosion and verbosity slopes per problem.
- **"Slows decay" (pre-registered, proposed):**
  - The factory's SCB erosion slope is at or below the control's on at least 2 of 3 problems.
  - The factory's verbosity slope is no worse than the control's on at least 2 of 3 problems.
  - The factory's strict-solved checkpoints are at least the control's minus 1.
  - With n = 3 this is a signal, not a significance claim.
- **Mapping to the builder runtime:** the "fresh container plus previous workspace" model maps to a builder session on the D189 read-only mirror. The base commit is the previous accepted workspace in a local-only scratch repo, the same pattern as `dedup-manager` (600 design §4.3). The machine run goes to Nano as a paste-ready prompt in `/mnt/project-files/let-em-loose/`, with PowerShell for anything he types.

## 4. Cost
**Phase A:** $0. No model calls; CPU time in a cloud session only.

**Phase B:** shadow dollars at list rates, using `1124-spend-estimate.md` §1 (builder session $2 / $5 / $12; lens calls a third Claude). This is subscription quota, not a bill (ADR-0021).

| Arm, 11 checkpoints | Builder sessions | Lens calls | Shadow $ low / expected / high |
|---|---|---|---|
| Factory | 11 / 22 / 50 | 50 / 180 / 400 | ~25 / ~115 / ~630 |
| Control | 11 / 11 / 11 | 0 | ~22 / ~55 / ~130 |
| **Total** | | | **~50 / ~170 / ~760** |

- Anchor: the paper's Opus 4.6 averages $3.17 per checkpoint, about $35 for 11.
- Proposed ceiling: `task_calls` 60 per checkpoint task. Pause and report to Nano if the first problem uses more than 150 calls or $60 shadow.
- The builder figures are unmeasured until the #1318 toy run's actual usage replaces them.

## 5. Asks for Nano
1. **Approve this design.** That also approves building Phase A now, ahead of #1320. D164 left the other bbp_factory Backlog cards for later, and this approval would start #705 early.
2. **Approve the dependency:** `tree-sitter-python` 0.25 in `rusty_erosion`, adding a new `Cargo.lock` entry.
3. **O1, Phase A sample:** (a) SCB reference solutions only (recommended); (b) also about 30 public Python repo histories.
4. **O2, Phase B comparison:** (a) factory plus control arm (recommended); (b) factory only, with the paper's aggregates as context.
5. **O3, D44 cleanups in the factory arm:** (a) off, matching the paper (recommended); (b) on, testing the factory's own claim about cleanups; (c) both, as a third arm at about +40% cost.
6. **The pre-registered thresholds** in §3 A.4 and §3 B, or your own.
7. **Phase B paid approval and ceiling:** not asked now. It comes back as its own ask once #1320 has run and Phase A has reported.

## 6. Drift noted
- Card #705 says "Hermes builder, Codex lens". D64 (2026-11-04, after the card) makes builders Claude CLI only, Codex is a lens, and SPEC names Hermes as the JARVIS face, not the engine. This design follows D64.
- The card says agreement shows `rusty_erosion` is "trustworthy on Rust". Phase A measures Python only (§3 A.5).

## 7. Owner answers
_(empty)_
