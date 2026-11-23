---
category: Security
changelog: Update transitive url and cookie dependencies to patched compatible releases.
---
## 2026-11-23 - Patch nightly dependency advisories

- **Security:** Update the workspace lockfile to url 2.5.9 and cookie 0.18.3 for RUSTSEC-2026-0592 and RUSTSEC-2026-0594. The url update requires form_urlencoded 1.2.3. Existing dependency constraints support these releases without new direct dependencies or advisory exceptions.
