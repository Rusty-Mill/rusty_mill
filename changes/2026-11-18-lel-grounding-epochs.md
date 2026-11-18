---
category: Fixed
changelog: The Let Em Loose grounding bundle scores only the current epoch's votes of a reopened claim and lists earlier-epoch votes apart.
---
## 2026-11-18 - Let Em Loose grounding bundle: reopened claims

- **Fixed:** `1166-grounding.sh` summed every vote that had scored when it was posted, so after a `reopen` (a new epoch) it counted the first epoch's votes too: smoke-6's claim 6 showed -16 against the board's -8. It now scores only the claim's current epoch, as the board does, and lists the votes of earlier epochs in their own block, so the audit still sees them. The section header shows the current epoch and the counts.
- **Known limitations:** tested on a throwaway `bbp` store with a claim dropped, reopened and dropped again (board -8, bundle -8, three earlier-epoch votes listed); not run on BaileyAi.
