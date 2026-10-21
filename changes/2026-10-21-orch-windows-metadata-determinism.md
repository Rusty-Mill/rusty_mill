---
category: Fixed
changelog: Stabilize change-impact Cargo metadata revisions across equivalent Windows path spellings.
---
## 2026-10-21 - Stable Windows impact revisions

- **Fixed:** Captured same-fixture metadata showed Cargo alternating ordinary and extended-length Windows spellings in package identities, manifests, and target paths. Change-impact hashing now canonicalizes those schema-owned paths while retaining meaningful dependency, feature, target, manifest, and opaque application metadata changes. Opt-in diagnostics remain bounded and content-free.
