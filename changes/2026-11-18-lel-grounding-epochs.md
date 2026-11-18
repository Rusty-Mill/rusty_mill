---
category: Fixed
changelog: The Let Em Loose grounding bundle scores only the current epoch's votes of a reopened claim and lists earlier-epoch votes apart.
---
## 2026-11-18 - Let Em Loose grounding bundle: reopened claims

- **Fixed:** `1166-grounding.sh` summed every vote that had scored when it was posted, so after a `reopen` (a new epoch) it counted the first epoch's votes too: smoke-6's claim 6 showed -16 against the board's -8. It now scores only the claim's current epoch, as the board does, and lists the votes of earlier epochs in their own block, so the audit still sees them. The section header shows the current epoch and the counts.
- **Fixed (review):** a reopen also wakes votes into the new epoch: the reopening evidence, or every dormant vote on a human reopen. The history records that only on the reopen's `Status` step (`woke`), while the vote's own step keeps its old epoch and `scoring`. The bundle now counts a vote listed in the current epoch's `woke` as scoring there (flag `woke-by-reopen`), so its per-author scores and mechanical hint match the board.
- **Known limitations:** tested on a throwaway `bbp` store with a claim dropped, reopened and dropped again (board -8, bundle -8, three earlier-epoch votes listed) and on two synthetic histories carrying `woke` (a human reopen waking a dormant vote: -10 = board -10; an evidence reopen waking its own evidence: -8 = board -8); not run on BaileyAi. A real `woke` needs an executed Support on a dropped claim with a closed dependency, which M1 cannot produce.
