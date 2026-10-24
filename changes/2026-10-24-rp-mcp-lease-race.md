---
category: Fixed
changelog: `rp-mcp` no longer sends two simultaneous calls to the same upstream connection while another is idle.
---
## 2026-10-24 - rp-mcp: simultaneous calls claim distinct upstream connections

- **Fixed:** `Pool::lease` picked the least-busy upstream connection and raised its count as two separate steps. Calls arriving together could both pick the same idle connection. Each connection runs one call at a time, so they queued behind each other while another connection sat unused. The claim now raises the count with a compare-and-swap that only succeeds if the count is still what was read, and a caller that loses picks again.
- **Found by:** `handler::calls_to_one_upstream_run_in_parallel_up_to_the_pool_size`. It failed in the merge queue on all three attempts, with a peak of 2 calls against a pool of 3 and about 1.0 s per attempt: two calls overlapped, and the third queued behind one of them.
- **Verified:** a new unit test, `simultaneous_claims_take_distinct_idle_slots`, releases three threads together 2,000 times. Each time they must take three distinct slots. It failed before the fix with `[0, 0, 1]` and passes after it. `cargo nextest run -p rp-mcp --all-features` passes.
