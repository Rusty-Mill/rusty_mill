---
category: Changed
changelog: Mill Board: the evidence backfill shows where each pull request is named, can check with GitHub that each merged, and skips cards that change while it runs.
---
## 2026-11-12 - Mill Board: evidence backfill, reviewable and safer

- **Changed:** the `--backfill-evidence` plan lists, per link, every place it is named, so wording such as "superseded by PR #5" is easy to spot.
- **Added:** opt-in `--check-merged` asks GitHub (`GET /repos/O/N/pulls/N`, `GITHUB_TOKEN`) whether each named pull request merged and keeps only those. Unmerged and missing ones are dropped and listed on stderr; any other lookup failure stops the run before anything is written.
- **Changed:** a card changed while the backfill runs is skipped and reported instead of aborting the run. Owner and repository names in a pull request URL use the same character check as the repository map, and `PR #0` is no longer taken as a pull request.
- **Known limitations:** stored evidence is not re-checked when read; that belongs with the card dialog showing it as links.
