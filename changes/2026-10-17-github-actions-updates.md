---
category: Changed
changelog: Update the pinned Rust toolchain, artifact upload and tool installer GitHub Actions across CI, baseline and release workflows.
---
## 2026-10-17 - Refresh pinned GitHub Actions

- **Changed:** update the full commit pins for `dtolnay/rust-toolchain`, `actions/upload-artifact` (v4.6.2 to v7.0.2, including Windows reports) and `taiki-e/install-action` (v2.87.26 to v2.87.28).
- Workflow triggers, permissions, action inputs and selected Rust versions are unchanged.
- **Known limitations:** this fragment does not establish execution of the manual baseline or release workflows.
