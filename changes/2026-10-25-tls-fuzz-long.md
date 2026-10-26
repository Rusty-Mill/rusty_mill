---
category: Added
changelog: "A manual workflow runs every `rusty_tls` fuzz target for hours, on the native or `ring` backend, for the evidence bar's long fuzz run."
---
## 2026-10-25 - rusty_tls long fuzz run workflow

- **Added:** `.github/workflows/tls-fuzz-long.yml`, run by hand. It runs one job per fuzz target, 3 hours of wall time each by default: a configured budget of 24 job-hours over the 8 targets, matching the evidence bar's "24 CPU hours" example (Tier 1 item 3). That is a budget, not measured CPU time. It runs on the native backend by default, which is the one ADR-0002 Amendment 2's consumer uses, or on `ring`. Before any fuzz job starts, `fuzz_seconds.py` refuses a duration outside 1 to 19,800 s; zero would mean no limit to libFuzzer. A crash or a target that runs nothing fails its job. Each target's full log streams to a file as it runs and is uploaded with any crashing inputs and the grown corpus, including after a cancellation or timeout.
- **Changed:** the fuzz package can build on either backend: its `ring` feature is the default, and `native` needs `--cfg rusty_tls_native_crypto`. `tls_fuzz_smoke.sh` takes `FUZZ_FEATURES` (cargo arguments for `run`) and `FUZZ_TARGETS` (run only those, each of which must exist), and streams each target's output to `FUZZ_LOG_DIR` instead of holding it in memory. The CI smoke job's printed output is unchanged.
