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
| P1 | `rsi-core`: domain types, accept gate, budget meter, search helpers, lineage hash chain | merged ([#472](https://github.com/Rusty-Mill/rusty_mill/pull/472)) |
| P2 | Task format, three toy tasks, sandboxed executor, out-of-process grader | this PR |
| P3 | Model broker, inner harness `a0` (AIDE0 behaviour) | planned |
| P4 | Outer loop, `rsi calibrate`, 10-step end-to-end run, `rsi report` | planned |

## Crates

| Crate | Purpose | ADR-0002 tier |
|---|---|---|
| [`rsi-core`](crates/rsi-core) | Pure domain and ports: no I/O, no async, no clock | S (workspace foundation crates only) |
| [`rsi-runtime`](crates/rsi-runtime) | Adapters: sandboxed executor, task directories, metrics, graders; the [toy task suite](crates/rsi-runtime/tasks) | S (workspace crates only) |
| [`rsi-cli`](crates/rsi-cli) | The `rsi` binary; so far only the internal `__sandbox` and `__grade` entry points | S (workspace crates only) |

## Invariants covered so far

- **Budget (2):** `CostMeter` refuses any operation once tokens, wall-clock
  or GPU time reach the limit; usage past the limit is still recorded.
- **Accept gate (3):** `screen` rejects a candidate that does not beat the
  incumbent. `confirm` accepts only on a fresh-seed grade that beats it by
  more than the calibrated margin, and refuses reused seeds.
- **Lineage (4):** entries carry results rather than grades, so the grade is
  always recomputed, and a repeated `(task, seed)` result is an error.
  `LineageEntry::new` replays the gate on that evidence and refuses a
  decision that does not follow from it. `verify_chain` detects edits,
  deletions and reordering.

- **Private isolation (1):** solutions run in a fresh directory with
  read access to system directories only (Landlock). A solution that
  tries to read a task's private labels gets `PermissionError`, whether
  during public scoring or private grading. Private labels are read only by
  the separate `rsi __grade` process, and the executor fails closed when
  the sandbox cannot be enforced.
- **Sandbox limits (5):** no internet sockets (seccomp; Unix sockets
  stay available for the P3 broker), no writes outside the work
  directory, and CPU, memory, file size and descriptor rlimits. A
  wall-clock kill takes down the whole process group.

The sandbox needs Linux with Landlock, and the toy tasks need `python3`.

## Test

```sh
cargo test -p rsi-core -p rsi-runtime -p rsi-cli
```
