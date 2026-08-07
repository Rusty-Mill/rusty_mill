# Portable Process & Filesystem Contract — Phase 0

One-page definition of what this runtime promises, and what it explicitly
does not, per host. Any adapter divergence not listed here is a bug, not a
design decision.

## Supported hosts

Windows (MSVC), Linux, macOS. x86_64 and aarch64.

## Baseline guarantees (v1)

| Primitive | Guarantee | Backing crate |
|---|---|---|
| Paths | Canonical, absolute, `/`-normalized-for-display paths; capability-scoped roots (no `..`-escape) | `cap-std` |
| Filesystem ops | Open/read/write/stat/list/create/remove within a scoped root | `cap-std` |
| File locking | Advisory exclusive/shared locks, best-effort on all hosts | `std::fs::File` (stable since 1.89) |
| Process spawn | Spawn with explicit argv/env/cwd, inherit or capture stdout/stderr, block until exit | `std::process` |
| Stdio / PTY | Byte-stream stdio always; interactive PTY where the capability model reports `pty: true` | `portable-pty` |
| Environment variables | Read/write current-process env as UTF-8 `HashMap<String, String>` | `std::env` |
| Standard directories | Per-OS config/cache/data dirs for a named app, deterministic | `dirs` |
| Errors | Structured `ContractError` — `PathEscape`/`NotFound`/`PermissionDenied`/`Unsupported` are stable categories; `Io` is the explicit fallback with the OS error retained as `source` for diagnostics only, never for callers to match on | `thiserror` |

## Capability model

Tools MUST query `Capabilities::detect()` before depending on a
non-baseline behavior. Known v1 fields:

- `symlinks` — conservative baseline, not a hard platform fact: `true` on
  Unix, `false` on Windows. Windows *can* create symlinks under Developer
  Mode or elevated privilege; this crate does not yet probe for that, so
  treat `false` as "not proven safe to assume," not "impossible."
- `unix_permissions` — false on Windows; POSIX mode bits are not emulated.
- `pty_win32_input_mode` — tracks the known `portable-pty` gap where
  `PSEUDOCONSOLE_WIN32_INPUT_MODE` / `PASSTHROUGH_MODE` are not passed
  through on the stock crate. Report `false` until we adopt or vendor the
  patched fork.
- `advisory_locking` — true everywhere `std::fs::File::lock`/`lock_shared`
  is available (stable since Rust 1.89); semantics are advisory only,
  never mandatory.

## Explicit non-goals (v1)

- `fork()` / `exec()` process-image replacement semantics.
- POSIX ownership (uid/gid) and permission-bit emulation on Windows.
- Transparent POSIX shell-script portability (`#!/bin/sh` scripts are out of
  scope; this is a Rust runtime contract, not a shell compatibility layer).
- Arbitrary Unix signal delivery. `ProcessRunner::run` is synchronous
  (spawn → capture → wait) and exposes no live handle to signal; `kill`/
  `terminate` for a running child or PTY session is deferred to a future
  live-child trait, not promised in this spike (see `PtyControl`'s
  lifecycle docs in `crates/contract`).
- Sandboxing/capability security in the WASI sense — this contract targets
  real dev-tool filesystem/process access, not a sandbox. (We deliberately
  did not build on WASI/wasmtime for this reason; see PR description.)

## Behavior matrix

Produced by `cargo test --workspace` across the CI OS matrix
(`.github/workflows/ci.yml`). Each primitive is one of:

- **supported** — identical behavior across all three hosts.
- **normalized** — behavior differs at the OS level but the adapter hides
  it (e.g. path separators, case sensitivity of `dirs` output).
- **unsupported** — capability absent on that host; callers must check
  `Capabilities::detect()` first.

Current matrix (updated as reference tools land):

| Primitive | Windows | Linux | macOS |
|---|---|---|---|
| Scoped fs ops (`stat-tool`) | supported | supported | supported |
| Symlinks | unsupported (no elevation) | supported | supported |
| Process spawn + stdio capture (`proc-runner`) | supported | supported | supported |
| Interactive PTY (`pty-shell`) | normalized (ConPTY, flag gap noted above) | supported | supported |
| Advisory file locking | supported | supported | supported |
| Standard dirs | normalized (`%APPDATA%` vs XDG vs `~/Library`) | supported | supported |

## Reference tools

Three tools exercise the three hardest primitives with minimal surface
area, all built against `contract` only:

1. `tools/stat-tool` — lists a directory and stats each entry through a
   scoped `FsRoot`.
2. `tools/proc-runner` — spawns a child process, captures stdout/stderr,
   reports exit status.
3. `tools/pty-shell` — opens an interactive PTY and spawns the host's
   default shell in it.

## Explicitly deferred prior art decision

WASI/the WASM component model is the industry's existing answer to "one
execution contract, per-host adapters." We are **not** building on
wasmtime/WASI for v1: dev tools need real fs/process access outside a
capability directory, which WASI intentionally restricts. Revisit if a
future phase wants sandboxed plugin execution — don't silently re-derive
WASI's design by accident.
