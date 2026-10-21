---
category: Fixed
changelog: Stabilize change-impact Cargo metadata revisions across equivalent Windows path spellings.
---
## 2026-10-21 - Stable Windows impact revisions

- **Fixed:** Captured same-fixture metadata showed Cargo alternating ordinary and extended-length Windows spellings in native manifest and target source paths. Change-impact hashing now canonicalizes genuine native Windows filesystem fields while leaving opaque package identities, non-Windows paths, and extended paths with verbatim trailing periods or spaces unchanged. Opt-in diagnostics remain content-free and provide bounded aggregate coverage before optional detail.
