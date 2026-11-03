---
category: Changed
changelog: Update taiki-e/install-action from 2.87.30 to 2.88.0.
---
## 2026-10-31 - Update install-action

- **Changed:** bump taiki-e/install-action from 2.87.30 to 2.88.0 in CI and long-fuzz workflows, retaining pinned action SHAs and the explicit cargo-fuzz version.
- **Known limitations:** CI-only smoke coverage does not demonstrate execution of the default nextest version.
