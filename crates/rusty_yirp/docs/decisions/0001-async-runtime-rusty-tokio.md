# ADR-0001: Async runtime — `rusty_tokio` confirmed, no fallback needed

- **Status**: Accepted
- **Date**: 2026-08-16
- **Phase**: 0 (blocking prerequisite — PLAN.md § Phased milestones)

## Decision

`sessionmgr` depends on **`rusty_tokio`** as its async runtime and its sole
process-spawn layer, pinned at rev
`6e6f18471749ee8327ce52e9d9b825e2e9e5e1a7`.

The documented fallback (plain `tokio`) is **not** needed and is not adopted.

## Why this phase exists

PLAN.md makes Phase 0 a blocking prerequisite specifically so the runtime
dependency is a *verified, freshly pinned, actually-building* thing before any
domain code is written on top of it — rather than inheriting
`rusty_prime_agent`'s older pin on trust. Risk list item 4 ("`rusty_tokio`'s
long-term availability/stability") exists to be converted into evidence here.

## What was actually verified

Toolchain on the machine that ran this check:

```
rustc 1.94.1 (e408947bf 2026-03-25)
cargo 1.94.1 (29ea6fb6a 2026-03-24)
host: x86_64-unknown-linux-gnu
```

A throwaway probe crate depending only on `rusty_tokio` at the fresh rev, doing
the three things this project's architecture actually requires of it — build a
runtime, construct a `process::Command`, and reach the `as_std_mut()` escape
hatch that `procutil::prepare_detached` depends on for
`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`:

1. `cargo build` — **clean**.
2. `cargo run` — **clean**, printed
   `phase0: rusty_tokio spawn ok, status=ExitStatus(unix_wait_status(0))`,
   i.e. a real spawn-and-wait through the runtime succeeded.
3. `cargo check --target x86_64-pc-windows-msvc` — **clean**. This is the
   target that actually matters for this project, and it compiles
   `rusty_tokio` plus its transitive `rustils` (`platform`,
   `platform-windows`, `winargv`), `rusty_std`, and `rusty_win32` crates for
   Windows without error.

Resolved dependency graph observed during the check (all pinned by rev, none
tracking a branch):

| Crate | Rev |
|---|---|
| `rusty_tokio` v0.2.0 | `6e6f1847` |
| `rustils` (`platform`, `platform-windows`, `winargv`) v0.27.0 | `ce9259d4` |
| `rusty_std` v0.1.0 | `3ab2361e` |
| `rusty_win32` v0.1.0 | `a128d758` |

### Correction against `rusty_prime_agent`'s pin

`rusty_prime_agent` pins `rusty_tokio` at `01e455ae`. This project deliberately
does **not** reuse that pin — per Phase 0's own wording — and takes
`6e6f1847` (current `main` at time of writing) instead, verified above rather
than assumed compatible.

### API correction found during the probe

`rusty_tokio::runtime` is a **private** module. The runtime type is re-exported
at the crate root as `rusty_tokio::Runtime`. Worth recording because it is the
kind of small mismatch that otherwise gets rediscovered once per crate.

## Limitation of this verification — stated, not hidden

This check ran on **Linux**, not on the Windows machine this project targets.
That means:

- **Proven**: the pinned rev resolves, and the full dependency graph *compiles*
  for `x86_64-pc-windows-msvc`.
- **Not proven**: that it *links* and *runs* correctly on real Windows.
  `cargo check` does not invoke the linker, and no Windows binary was executed.

This is a genuine gap in Phase 0's exit criterion as PLAN.md states it ("pinned
dependency builds" — against "this machine's real toolchain", which the plan
assumed would be the Windows dev box). The gap is narrow: `rusty_prime_agent`
ships this exact dependency on Windows today, including Windows `AF_UNIX` and
detached spawn, which is strong prior evidence. But it is prior evidence, not
this project's own measurement.

**Action for the Windows dev box**: run `cargo build --workspace` and
`cargo test --workspace` there once, and append the result to this record. Until
that happens, treat "builds on Windows" as inherited-from-sibling-project
confidence rather than as locally verified.

## Consequences

- Phase 1 is unblocked; domain code may now be written.
- The pin lives in the workspace root `[workspace.dependencies]` so every crate
  shares one rev, and bumping it is a single deliberate edit — matching
  `rusty_prime_agent`'s and `rusty_tokio`'s own stated convention of bumping a
  rev deliberately and never tracking a branch.
