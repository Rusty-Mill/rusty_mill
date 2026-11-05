---
category: Added
changelog: Each lens's track record is scored per D84 and shown in its context: points over its blind reviews, with pivotal reviews scored only against ground truth (#599).
---
## 2026-11-05 - Lens track records

- **Added:** `lens run` saves each blind review job a lens answers (and each reply the protocol refused, with nothing posted) to the orch-store `lens_record` table (D58, D92). Each pass reads every lens's record from those rows and each claim's `bbp why --claim` history, and shows it in the lens's context as `record: P points over N blind reviews (S scored, V pivotal)`. Points are computed when read and never stored.
- **Scoring (`orch-lens::record`, D84, D92):** a job scores on its claim's latest settlement, at `±(1 + contested + 2·upset)`. A job whose own items decided the accept or drop line is pivotal: it is scored against ground truth where the claim has one (a failing `golden:` set), and is otherwise unscored and counted as pivotal. A human `decide` or `choose`, including a later reversal, is never pivotal. Voided and superseded claims leave the record. The tally also reports pending, pivotal and pivotal-with-ground-truth counts, and whether calibration is due (20 scored).
- **Changed:** the lens `Board` port returns the posted message ids, and a round's report lists its blind review jobs.
- **Known limitations:** BBP's history doesn't carry the line's `min_authors`, so a job whose removal also removes its lens as an author counts as pivotal. A passed golden set isn't visible in the history, so only a failing one is ground truth, and a job that posted executed evidence gets none (OPEN_QUESTIONS Q23). There's no report command yet (Q22).
