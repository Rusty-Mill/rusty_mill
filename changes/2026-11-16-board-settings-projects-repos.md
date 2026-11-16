---
category: Added
changelog: Mill Board Settings splits Repositories from Projects, and links a project to a repository.
---
## 2026-11-16 - Mill Board: Projects and Repositories pages, project links

- **Added:** Settings has separate Repositories (add, remove, GitHub slug) and Projects (status, linked repositories and their paths) tabs. A project is linked to another repository, or unlinked, with one click through the new `settings_project_link` operation (admin; also an MCP tool), which validates paths and keeps the default repository.
