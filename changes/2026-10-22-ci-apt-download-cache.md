---
category: Added
changelog: Cache authenticated Ubuntu package downloads for shared GitHub-hosted Linux CI setup.
---
## 2026-10-22 - Reuse verified Ubuntu package downloads

- **Added:** an isolated `.deb` download cache with official immutable cache actions, release/architecture/package-list/daily keys, trusted main-sweep writers, authenticated APT SHA-256 verification and checked download retention. Every hit still updates indexes and installs the unchanged package list normally; setup limits and required gates are preserved.
- **Known limitations:** direct mingw/valgrind installs are outside this shared-action cache. Real cold/warm GitHub performance remains pending trusted postmerge main execution; offline fixtures do not establish speedup.
