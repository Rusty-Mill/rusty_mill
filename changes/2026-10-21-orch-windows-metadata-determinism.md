---
category: Fixed
changelog: Stabilize change-impact Cargo metadata revisions across equivalent Windows path spellings.
---
## 2026-10-21 - Stable Windows impact revisions

- **Fixed:** Change-impact reports now retain the same Cargo metadata revision for semantically identical Windows workspace metadata while continuing to detect meaningful dependency, feature, target, manifest, and application metadata changes.
