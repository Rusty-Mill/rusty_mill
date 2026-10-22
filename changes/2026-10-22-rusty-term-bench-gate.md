---
category: Added
changelog: CI gates rusty_term parser+grid throughput against per-workload floors (rusty-term-bench job, bench/floors.txt).
---
## 2026-10-22 - rusty_term throughput floor in CI

- **Added:** a `rusty-term-bench` CI job. It runs on every PR that affects `rusty_term_core` or `rusty_term`, and is listed in `required-gate`. It builds `bench_vt_throughput` in release mode, generates the quick workloads, and fails if any workload's MB/s drops below its floor in `crates/libs/ui/rusty_term/bench/floors.txt`. The measured-vs-floor table goes to the job summary, so each run records its numbers.
- **Added:** `bench_vt_throughput --floors FILE`, with unit tests for floor parsing and comparison.
- **Changed:** the bench README now describes the job (it referred to a `bench-smoke` job that did not exist) and the ratchet rule: a speed-up raises its floor in the same PR.
- **Known limitations:** wall-clock MB/s on shared runners. Floors sit at about half the measured value, so only large regressions (roughly 2x) fail. Linux only.
