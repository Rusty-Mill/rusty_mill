---
category: Fixed
changelog: The Reviewer's post-merge check now waits for a merge-queued PR to reach MERGED and names only valid gh fields, so it no longer halts the run.
---
## 2026-10-15 - Reviewer confirms a queued merge without halting

- **Fixed:** the post-merge step in `run.md` and the `super-review` skill said to expect `MERGED` straight after the gate. With a merge queue the PR is still OPEN then, so the lane agent invented a `gh pr view --json isInMergeQueue` read, which is not a valid field, and the run halted after PR #762 had merged. Both docs now say to repeat `gh pr view --json state,mergeCommit` for up to 10 minutes and, if the PR is still OPEN, to leave the card in Review and report "queued, not yet merged" without halting.
- **Known limitations:** a card left in Review as queued is not moved to Done until a later wave confirms the merge.
