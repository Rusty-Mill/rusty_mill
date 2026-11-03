---
category: Added
changelog: rusty_bbp holdout suite versions (#597 slice 2) - host holdout_extended for pure additions, human holdout_updated for changes.
---
## 2026-11-03 - BBP holdout suite versions (Let Em Loose #597, slice 2)

- **Added:** the host action and event `holdout_extended(digest)` for a holdout suite version that only adds files, and the human action and event `holdout_updated(digest)` for one that removes or changes existing tests (owner answer G3). Both are refused on a task without a holdout suite (`holdout_mismatch`) or for the current digest (`duplicate`).
- **Changed:** after a version change, reports under the old digest are refused, every open claim with a candidate needs a fresh holdout pass, and old holdout failures leave `failing(c)` since that pass re-runs what the new suite keeps. Accepted claims stay accepted.
- **Changed (review):** a retired digest is never adopted again (`duplicate`), so an earlier suite's passes cannot count once more; and every claim with a candidate is marked changed at a version event, so `claims_delta` subscribers see that it is due (CL-93).
- **Added:** traces CL-77 and CL-93.
- **Known limitations:** the file-level "only additions" check and a CLI to write either action come with the host's holdout set file (a later slice).
- **Fixed (re-review):** after a suite version, an endorsement never unlocks a holdout pass reported under the retired suite (CL-98).
