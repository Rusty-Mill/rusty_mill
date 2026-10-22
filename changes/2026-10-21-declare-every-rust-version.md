---
category: Changed
changelog: "Every workspace crate now declares a `rust-version`, and CI fails a new crate that omits one. 132 crates were undeclared; each now declares the highest floor of its dependencies, its edition, and the workspace baseline 1.75."
---
## 2026-10-21 - every workspace member declares a rust-version (issue #846)

- **Declared:** 132 crates had no `rust-version`, so neither `check_rust_versions.py` nor `msrv-compile` (#809, #825) covered them. Each now declares the highest of: the workspace baseline (1.75), 1.85 for an edition-2024 crate, and the declared floor of anything in its normal and build dependency closure (a crate that depends on another undeclared one takes the floor computed for it). Resulting floors: 1.75 (43 crates), 1.77, 1.81, 1.85, 1.87, 1.88, 1.89, 1.91, 1.94.
- **Enforced:** `check_rust_versions.py` fails, naming them, when any workspace member declares nothing. The dependency-policy job already runs it, so a new crate must declare a floor.
- **Verified by:** the existing dependency-closure check (passes) and `msrv-compile`, which now builds these crates at their declared floors.
- **Closes:** #846.
