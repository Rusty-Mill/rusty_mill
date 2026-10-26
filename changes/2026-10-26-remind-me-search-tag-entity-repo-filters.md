---
category: Added
changelog: **`rusty_remind_me`** `remind_me_search` filters by tags (all or any), entity (name or alias) and git repository, before the limit applies.
---
## 2026-10-26 - remind-me: search by tag, entity and repo

- **Added:** `remind_me_search` takes `tags` with `tag_mode` (`all` by default, or `any`), `entity` (resolved by name or alias; matches memories linked to it or naming it as subject or object) and `repo` (a git remote in any URL form, compared after normalization). They narrow both halves before their candidate limits: the keyword query filters before it takes its rows, and the semantic scan admits rows best first until its pool is full, so a scoped search is not short-filled. Scope and sensitivity now use the same semantic admission, so they no longer short-fill it either.
- **Fixed:** `tags` was already accepted by the search input but silently ignored, including by saved searches built from it. Saved searches with tags now narrow as saved.
- **Known limitations:** the expansion sections (`expand_entities`, `include_neighbors`, `expand_co_retrieval`) are not narrowed. An unknown entity returns no results rather than an error. Memories written before provenance capture have no `git_remote` and never match `repo`.
