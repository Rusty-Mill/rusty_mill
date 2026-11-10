---
category: Changed
changelog: rusty_term `bench_vt_throughput` prints best and median per-pass MB/s per workload; gen_workloads.py writes its combining marks as escapes.
---
## 2026-11-10 - Best-of-N and median MB/s in bench_vt_throughput

- **Changed:** `bench_vt_throughput` times each pass and prints best and median MB/s per workload on a new indented line, beside the aggregate. The aggregate line, `--once` and the `floors.py` / `ir_gate.py` parsing are unchanged. `bench/README.md` says to quote best-of-N (N ≥ 20) in PRs.
- **Fixed:** `gen_workloads.py` wrote the `unicode_heavy` accents as raw decomposed (NFD) literals, which an NFC-normalizing editor could silently change. They are now `\u03xx` escapes; the generated workloads are byte-identical.
