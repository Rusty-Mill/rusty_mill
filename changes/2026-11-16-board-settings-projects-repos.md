---
category: Added
changelog: Mill Board Settings splits Repositories from Projects, links a project to a repository, and adds a deferred project status.
---
## 2026-11-16 - Mill Board: Projects and Repositories pages, project links, deferred status

- **Added:** Settings has separate Repositories (add, remove, GitHub slug) and Projects (status, linked repositories and their paths) tabs. A project is linked to another repository, or unlinked, with one click through the new `settings_project_link` operation (admin; also an MCP tool), which validates paths and keeps the default repository.
- **Added:** project status `deferred` (parked for later): off the default views, refuses new cards, hands out no work.
- **Known limitations:** `deferred` behaves like `paused` today; they differ in meaning only.
