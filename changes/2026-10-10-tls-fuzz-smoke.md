---
category: Added
changelog: CI job `rusty-tls-fuzz` runs every `rusty_tls` fuzz target for 30 s whenever the TLS engine jobs run (blocking, in `required-gate`); the planner now also selects those jobs when their own CI files change.
---
## 2026-10-10 - CI: fuzz smoke for the native TLS engine

- **Added (CI):** job `rusty-tls-fuzz` runs each of the 8 `rusty_tls` fuzz targets for 30 seconds wherever the `tls_engine` jobs run (a change to `rusty_tls` or a dependent crate, the engine's own CI files, and full runs). Pinned dated nightly (`FUZZ_TOOLCHAIN`), `cargo-fuzz` 0.13.2, host triple passed explicitly. Blocking and in `required-gate`. The logic is `tls_fuzz_smoke.sh`, tested offline against a fake `cargo-fuzz` and against the real command construction: a crash, a target that runs nothing, a failing target list and fewer than 8 targets each fail it.
- **Changed (CI planner):** `tls_engine` is also selected when `ci.yml`, `ci_plan.py`, `tls_fuzz_smoke.sh` or `live_internet_check.sh` change. A skipped job reports success, so a CI-only PR that edited the engine jobs never exercised them. Costs about 10 minutes of extra CI on a `ci.yml` edit.
- **Added (docs):** `docs/research/TLS-EVIDENCE-BAR.md`, the two-tier evidence bar proposal. Tier 1 (one opt-in consumer) approved by the owner on 2026-10-10 with the fuzz length left to the reviewer; Tier 2 not approved.
- **Verified:** all 8 targets clean locally and on a GitHub runner (run 38094570843, about 5.5 minutes with the build). The first runner run failed all 8: the prebuilt `cargo-fuzz` is musl and defaulted to a musl build the runner has no std for.
- **Known limitations:** 30 s per target finds shallow regressions only. No target for TLS 1.3 handshake messages or the 1.3 machines beyond version negotiation. The long run the evidence bar needs is not done, so bar item 3 is half met.
