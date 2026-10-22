---
category: Added
changelog: CI gates rusty_term parser+grid instructions per byte (cachegrind, deterministic) against per-workload ceilings; MB/s is reported for information only.
---
## 2026-10-22 - rusty_term instruction-count gate in CI

- **Added:** the `rusty-term-bench` CI job. It runs on every PR that affects `rusty_term_core` or `rusty_term` and is listed in `required-gate`.
  - It counts the instructions of one `bench_vt_throughput --once` pass per quick workload under valgrind's cachegrind (`bench/ir_gate.py`).
  - It fails if instructions per input byte exceed the workload's ceiling in `crates/libs/ui/rusty_term/bench/ir_ceilings.txt`.
  - glibc's memmove variant is pinned with `GLIBC_TUNABLES`, so the count does not depend on the runner's CPU.
  - Wall-clock MB/s goes to the job summary for information and never fails the build.
- **Added:** `bench_vt_throughput --once` feeds each file once with no timing, for the instruction counter. `ir_gate.py` has unit tests, run in the same job.
- **Changed:** the bench README now describes the job (it referred to a `bench-smoke` job that did not exist) and the ratchet rule: a PR that lowers a count lowers its ceiling in the same PR.
- **Known limitations:**
  - Ceilings carry 10% headroom for toolchain codegen drift, so smaller regressions pass.
  - x86-64 Linux only.
  - A toolchain bump may need the ceilings re-measured.
