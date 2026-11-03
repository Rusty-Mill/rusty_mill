---
category: Added
changelog: remind-me checks a saved decision against standing decisions and returns likely conflicts; `remind_me_add` takes `supersedes`.
---
## 2026-10-26 - remind-me: contradiction check on save for decisions

- **Added:** `remind_me_add`, and a `remind_me_update` that changes content or type, now check a decision against the standing decisions in the same project (live, not sensitive, not reverted, abandoned or superseded) that make the same claim or share an entity mentioned by at most 20 memories. A shared entity also needs a word overlap (Jaccard) of at least 0.3. Up to 5 matches come back as `contradiction_candidates` with id, score, reason and shared entities. The check never blocks the write and never supersedes anything.
- **Added:** `remind_me_add` takes `supersedes`, a list of live memory ids that the new memory replaces. The add and its retirements are one transaction: any id that is missing, deleted, already superseded or a project claim refuses the whole add, and nothing is written.
- **Fixed:** the `remind_me_add` schema no longer says a new triple supersedes a live one with the same subject and predicate. Add has never done that; `supersedes` now does it explicitly.
- **Known limitations:** if the check fails, the write still commits. The error goes to stderr and is returned as `contradiction_check_error`. Entity links are indexed by entity only, so finding a memory's own entities is a full scan of the links. On a synthetic 50k-memory, 100k-link store the check took about 80 ms in a release build.
