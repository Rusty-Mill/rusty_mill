---
category: Added
changelog: Mill Board's projects registry can now hold repositories as their own entity and projects that span repositories.
---
## 2026-11-14 - Mill Board: projects registry v2

- **Added:** version 2 of the projects file (`repos` with an optional `github` slug, `projects` with per-repository paths and a `default_repo`), `Projects::repos_of` and `paths_in`. Older files parse unchanged and are still written in the older form unless a project spans repositories or a repository has a slug, so a rollback binary can read them.
- **Known limitations:** the settings UI and `settings_projects_set` still handle one repository per project and refuse a listing that spans repositories; that follows in the next PR of the design.
