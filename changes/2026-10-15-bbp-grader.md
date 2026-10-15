---
category: Added
changelog: **BBP blind rubric grader.** `bbp grade pack` writes the specs and frozen rubric items under neutral ids in hash order with the arm key kept apart, and `bbp grade join` opens the key only once every id is scored, reporting `rubric_ok` per run and `lost_rubric_items` per pair.
---
## 2026-10-15 - BBP blind grader

- **Added:** `bbp grade pack --manifest M --rubrics D --seed S --out O` writes `pack.json` (what the grader sees: neutral id, spec text, rubric items) and `key.json` (not given to the grader). `bbp grade join --key K --scores S --rubrics D` joins the grader's per-item verdicts.
- **Blind by construction:** the pack has no pair, arm, run, role or log fields; ids are a hash of the seed and the run, so identical specs in two arms differ; order is by hash of the seed and id, so position says nothing. Tests assert the pack text contains none of the pair names, `control`, `treatment`, `approved`, `unused` or run paths, and that ids and order do not depend on input order.
- **Sealed key:** `join` refuses unless the scores cover every id with one verdict per rubric item, and refuses ids not in the pack. The grader is named in the scores file and carried to the output; the output includes the sha256 of the committed rubric files so a run graded against different rubrics shows.
- **Graded:** both arms' approved specs, and the treatment arm's unused versions (from the attribution), so `lost_rubric_items` = rubric items an unused version met and the approved spec did not. Null pairs are not graded. A rubric is `<rubrics>/<pair id>.json` with `items` and `pass_mark` (1 to the item count); the pass mark is inclusive.
- **Limits:** blindness is mechanical, not absolute: a spec that names its author or arm in prose is not scrubbed. The manifest loader is not covered by an end-to-end test; the pure pack and join are. `rubric_ok` in the pairs manifest is now optional (absent reads as not met) so one manifest serves packing and, after grading, `bbp pairs`.
