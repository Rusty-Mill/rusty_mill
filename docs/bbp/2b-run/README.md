# BBP 2b evidence run: inputs

Status: **approved by the owner on 2026-10-21 and frozen.** These are the briefs,
classification, rubrics and pass marks the criteria
(`docs/bbp-concurrency-design.md`, "2b gate criteria") require to be committed
before the first run; a run whose set differs is void. Nothing here authorizes
a run: the pilot and its spend need the owner's separate go.

| File | What |
| --- | --- |
| `tasks.json` | per task: kind (separable / shared), the two concerns, section ownership (`A`, `B`, `AB`), which concern each rubric item needs, null-pair flag (p01 to p03) |
| `tasks/pNN/brief.md` | the brief the task is opened with (`bbp open --brief`) |
| `tasks/pNN/pNN.rs` | the failing test to commit as `tests/pNN.rs` in that task's target crate (see `crates/apps/rusty_bbp_host/docs/proving-run.md`, step 2) |
| `rubrics/pNN.json` | the frozen rubric and pass mark, one per pair id; `bbp grade --rubrics rubrics/` hashes every `.json` here, so nothing else lives in it |
| `pilot/` | the uncounted pilot task: brief, test, rubric |
| `roster-control.json` | the proving roster, unchanged (N=1, `--live-turns 1`) |
| `roster-treatment.json` | the same plus `planner2`, a copy of `planner` (N=2, `--live-turns 2`) |
| `agents-control.json`, `agents-treatment.json` | the proving launchers, each pinned to one priced model, `--setting-sources ""`, `--disallowedTools Agent,Task`, and told that `stale_turn` ends the turn. In the treatment, `planner` owns concern A and `planner2` concern B |

Classification against the criteria's definitions: p01 to p05 are separable
(every rubric item needs one concern; the concerns own disjoint sections);
p06 to p10 are shared (every section is owned by both concerns, so 100% of
rubric items need a co-owned section, above the 30% and 2-item line).

Every task's tests were checked to pass against a private reference
implementation, so each is passable as briefed.

## Order

1. `bbp agents-check --control agents-control.json --treatment agents-treatment.json --criteria ../2b-criteria.json`.
2. Pilot, uncounted, on a dedicated API key: one N=1 pair on `pilot/`, then
   `bbp tokens --api-usage FILE` reconciles within 2% or the run stops.
3. The owner approves `C = 3 x` the pilot's N=1 cost per run and sets the key's limit to `23 x C`.
4. Phase 1: the six null runs (p01 to p03 twice at N=1); stop if any null ratio is outside 0.85 to 1.15.
5. Phase 2: seven more controls and ten treatments; then `bbp grade` and `bbp pairs`.
