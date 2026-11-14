---
category: Added
changelog: Mill Board settings can save projects that span repositories and each repository's GitHub name.
---
## 2026-11-14 - Mill Board: settings for spanning projects (repos apart from projects, PR 3 of 5)

- **Added:** `settings_projects_set` takes a `registry` (the version 2 projects file) beside the older `repos` listing; `settings_get` returns the current `registry`. In Settings > Repositories a project given the same name under two repositories spans both (its first repository stays the default), and each repository has an optional GitHub `owner/name`. A save is refused while a card names a repository its project would no longer have.
- **Known limitations:** the Settings browser check is flaky about 1 run in 4 on `main` too (the dialog sometimes does not open); a spanning project's status is one setting shown under each repository.
