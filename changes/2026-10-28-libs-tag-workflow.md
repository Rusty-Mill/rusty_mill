---
category: Added
changelog: Add a `libs-*` tag workflow so repositories outside the monorepo can pin its crates (ADR-0013).
---
## 2026-10-28 - Tag libs for outside repositories

- **Added:** `.github/workflows/libs-tag.yml`, run by hand, runs `.github/scripts/libs_tag.sh` (tested against scratch repositories in `test_libs_tag.py`), which tags the head of `main` as `libs-YYYY.MM.N`. Repositories outside the monorepo pin its crates to one such tag through Cargo git dependencies (`docs/adr/0013-consume-monorepo-crates-by-libs-tag.md`). The first consumer is rusty-remind-me's own repository (Mill Board #61).
- **Known limitations:** the monorepo's CI does not build outside consumers; a consumer finds a break when it bumps its tag.
