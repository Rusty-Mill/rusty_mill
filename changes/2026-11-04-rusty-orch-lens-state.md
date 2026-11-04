---
category: Added
changelog: "`rusty_orch lens run` saves each board's scheduler memory, so a restart does not re-ask passes already made."
---
## 2026-11-04 - rusty_orch: the lens runner's memory survives a restart

- **Added:** `orch_lens::saved`: `State::saved` turns the scheduler's memory into plain data and `State::restore` rebuilds it, re-checking every provider id and refusing a repeated entry (#594). Jobs in flight are not kept, since no job outlives its process.
- **Added:** `orch_store::Store::load_lens_state` and `save_lens_state`, one `LensStateRecord::v1` per BBP board in a new `lens-states.mmap` beside the meters. A new file, so nothing existing is migrated.
- **Changed:** `rusty_orch lens run` keys the memory by board: the canonical `--dir` and the `--task` id (`lens::run::board`), because BBP task ids are scoped to their store directory. A task of the same name in another BBP directory starts with nothing remembered; the same task resumes. A directory whose canonical path is not UTF-8 is refused before any state is read, since a lossy key could name two directories. It loads the board's memory at start and saves it after every lens's round; a failed save ends the run with an error. `passes` takes the `keep` callback. A crash mid-round asks that round's jobs again; the meter already charges any call it had reserved.
- **Changed:** `orch-lens` cards, jobs and each lens's progress carry the claim's BBP `epoch`. A lens's passes count only in the epoch they were made in, so a claim reopened between runs (or during one) is reviewed again, blind first, instead of being remembered as done. A candidate's attack passes are counted per epoch too (`schedule::Attacks`), so a reopened candidate's old count cannot hold its new review back under the fair-attacks rule.
