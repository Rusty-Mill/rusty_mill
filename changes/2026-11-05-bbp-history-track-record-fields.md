---
category: Changed
changelog: `bbp why --claim` adds `min_authors` on line rules, evidence refs, reopen wakes, candidate and holdout report steps, and the failing set's candidate; orch's lens track records read them and drop D92's four interim limits (#599, D112).
---
## 2026-11-05 - Claim replay carries what track records need

- **Changed:** `bbp why --claim` (`rusty_bbp::claim_history`) now shows more, as a view only with no log or schema change (#599, D112): `min_authors` on `AcceptLine` and `DropLine`, the refs on each evidence step, the items each reopen woke (`woke`, left out when empty), a `Candidate` step when a build candidate becomes current, the candidate on each `Failing` step, and a `Report` step for each final holdout report that speaks for the current code, with its passed and failed profiles.
- **Changed:** lens track records read those fields, so D92's four interim limits end. A line's author minimum is the task's. A reopen counts exactly the items it woke. Golden truth works on the `bbp why` path: the latest golden result on the candidate current at settlement, passed meaning accept was right and failed meaning drop was right. A job is not judged by a report it cites. Fewer jobs are counted pivotal-unscored as a result.
- **Known limitations:** the record needs a `bbp` built with this change; an older one's history is refused with "line has no min_authors" rather than read conservatively.
