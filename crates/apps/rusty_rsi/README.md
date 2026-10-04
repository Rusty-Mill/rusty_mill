# rusty_rsi

An AIDE²-style recursive self-improvement harness. An inner agent improves a
task's solution against a public score under a fixed budget. An outer agent
rewrites the inner agent's code and keeps a rewrite only if its grade (the
mean private, held-out score across a task suite) beats the incumbent's by
more than the measured noise.

Design, reuse decisions and scope: [ADR-0005](../../../docs/adr/0005-rsi-harness.md).

## Status

| Phase | Deliverable | State |
|---|---|---|
| P1 | `rsi-core`: domain types, accept gate, budget meter, search helpers, lineage hash chain | this PR |
| P2 | Task format, three toy tasks, sandboxed executor, out-of-process grader | planned |
| P3 | Model broker, inner harness `a0` (AIDE0 behaviour) | planned |
| P4 | Outer loop, `rsi calibrate`, 10-step end-to-end run, `rsi report` | planned |

## Crates

| Crate | Purpose | ADR-0002 tier |
|---|---|---|
| [`rsi-core`](crates/rsi-core) | Pure domain: no I/O, no async, no clock | S (workspace foundation crates only) |

## Invariants covered so far

- **Budget (2):** `CostMeter` refuses any operation once tokens, wall-clock
  or GPU time reach the limit; usage past the limit is still recorded.
- **Accept gate (3):** `screen` rejects a candidate that does not beat the
  incumbent. `confirm` accepts only on a fresh-seed grade that beats it by
  more than the calibrated margin, and refuses reused seeds.
- **Lineage (4):** entries carry results rather than grades, so the grade is
  always recomputed; `verify_chain` detects edits, deletions and reordering.

Private isolation (1) and sandbox limits (5) arrive with the executor in P2.

## Test

```sh
cargo test -p rsi-core
```
