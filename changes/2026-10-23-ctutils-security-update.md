---
category: Security
changelog: Update locked ctutils to 0.4.4 to fix RUSTSEC-2026-0385.
---
## 2026-10-23 - Patch ctutils constant-time operations

- **Security:** Update the root lockfile from ctutils 0.4.2 to patched 0.4.4 to fix non-constant-time mask generation and `u128` comparisons (RUSTSEC-2026-0385).
- **Known limitations:** No parent dependency versions or manifests change; this update relies on the upstream constant-time fixes.
