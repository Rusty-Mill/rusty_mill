---
category: Changed
changelog: Mill Board: the evidence backfill shows where each pull request is named and skips cards that change while it runs.
---
## 2026-11-12 - Mill Board: evidence backfill, reviewable and safer

- **Changed:** the `--backfill-evidence` plan lists, per link, every place it is named, so wording such as "superseded by PR #5" is easy to spot. Nothing checks that a pull request merged.
- **Changed:** a card changed while the backfill runs is skipped and reported instead of aborting the run. Owner and repository names in a pull request URL use the same character check as the repository map, and `PR #0` is no longer taken as a pull request.
- **Known limitations:** stored evidence is not re-checked when read; that belongs with the card dialog showing it as links.
