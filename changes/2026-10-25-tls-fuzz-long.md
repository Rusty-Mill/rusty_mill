---
category: Added
changelog: "A manual workflow runs every `rusty_tls` fuzz target for hours, on the native or `ring` backend, for the evidence bar's long fuzz run."
---
## 2026-10-25 - rusty_tls long fuzz run workflow

- **Added:** `.github/workflows/tls-fuzz-long.yml`, run by hand. It runs one job per fuzz target, 3 hours each by default, which is 24 CPU hours over the 8 targets (evidence bar, Tier 1 item 3). It runs on the native backend by default, which is the one ADR-0002 Amendment 2's consumer uses, or on `ring`. A crash or a target that runs nothing fails its job. The log, any crashing inputs and the grown corpus are uploaded.
- **Changed:** the fuzz package can build on either backend: its `ring` feature is the default, and `native` needs `--cfg rusty_tls_native_crypto`. `tls_fuzz_smoke.sh` takes `FUZZ_FEATURES` (cargo arguments for `run`) and `FUZZ_TARGETS` (run only those, each of which must exist). The CI smoke job is unchanged.
