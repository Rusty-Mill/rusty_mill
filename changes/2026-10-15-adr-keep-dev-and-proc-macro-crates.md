---
category: Changed
changelog: ADR-0010 records the decision to keep criterion, proptest, tokio-test, libfuzzer-sys, syn, quote and proc-macro2 external, with revisit conditions; documentation only.
---
## 2026-10-15 - ADR-0010: keep dev-only and proc-macro crates external

- **Changed (docs):** `docs/adr/0010-keep-dev-only-and-proc-macro-crates.md` records keep-or-replace for `criterion`, `proptest`, `tokio-test`, `libfuzzer-sys`, `syn`, `quote` and `proc-macro2`, with the reason for each and the conditions that would reopen it. Closes #707.
- **Known limitations:** the usage counts are a grep of workspace manifests, not a vetted audit. No code changed.
