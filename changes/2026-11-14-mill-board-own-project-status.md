---
category: Fixed
changelog: Mill Board: Settings > Repositories can set the status of a repository's own project.
---
## 2026-11-14 - Mill Board: a status control for a repository's own project

- **Fixed:** Settings > Repositories drew a status control only for projects listed inside a repository, so a repository's own project (for example `apprentice_ai`, which is the whole of its repository) could not be paused, set to experiment or archived from the screen. It now has the same control. The server already accepted any known project. The Inbox still has none.
- **Known limitations:** metrics and the roadmap still count every project.
