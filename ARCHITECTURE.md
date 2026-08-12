# Architecture

## Overview

`rustils_async` is a native-async sibling to [`rustils`](https://github.com/baileyrd/rustils),
adding async and multithreading support to the platform-abstraction layer that
`rusty_foundation_akb` requires. It is not a fork: it depends on `rustils`'
`platform`/`platform-mock`/`platform-linux` crates (pinned git dependencies) for
their data types and, where sound, their existing sync implementations, and adds an
async trait surface and a real async wait path alongside them. Currently scoped to
the `process` domain only — see `docs/adr/0001-native-async-rustils.md` for why.

Non-goals: this is not a general-purpose async runtime (it deliberately depends on
no external one — see `RM-ASYNC-RUNTIME-0001` in the Rusty-Mill Foundation AKB), not
a competitor to `tokio`/`async-std`, and not a full port of every `rustils` domain —
fs/net async and the Windows/BSD backends are reserved, not built (see the README's
"Reserved, not built" table).

## Boundaries

Ports-and-adapters: `reactor-core` and `platform-async` define the ports (traits and
primitives, no I/O, no unsafe); `platform-async-linux`/`platform-async-mock` are the
adapters. Nothing above the port depends on a specific backend.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `reactor_core::Clock` | `SystemClock` | explicit time source, substitutable for deterministic tests |
| `platform_async::process::AsyncSpawner` / `AsyncChild` | `platform-async-linux::AsyncLinuxSpawner` (real, pidfd+epoll), `platform-async-mock::AsyncMockSpawner` (scripted, resolves immediately) | Windows/BSD adapters are reserved rows, not built yet |
| `platform::process::Spawner` (sync, from `rustils`) | `platform-linux::LinuxSpawner` | reused directly for spawning itself — only the *wait* is made async (see `platform-async/src/process.rs`'s module doc comment) |
| `threading::Mutex`/`RwLock` | wraps `std::sync::Mutex`/`RwLock` | adds an explicit, chosen-at-construction `PoisonPolicy` instead of leaving poisoning handling to each call site |

## Structure

Modular monolith: one Cargo workspace, six crates, each with one coherent
responsibility (`reactor-core` primitives, `platform-async` trait surface,
`platform-async-mock`/`platform-async-linux` backends, `threading`, and
`coreutils-async` as the reference consumer). No crate has been split into a
separate repository — `rusty_async` (a sibling repo intended for shared async
primitives across the wider Rusty-Mill ecosystem) stays empty until a second real
consumer beyond this repo exists, per the standing "extract only for a concrete
forcing function" principle (Atlas `ATLAS-DEP-0010`).

## Data flow

`arun <program> [args]` (in `coreutils-async`) is the concrete walkthrough:

1. `AsyncLinuxSpawner::spawn` calls straight through to `platform-linux`'s sync
   `Spawner::spawn` — process creation is a single fast syscall, not something
   async multiplexing helps with.
2. The returned `AsyncLinuxChild::wait()` opens a `pidfd` for the child and awaits
   a `PidfdReady` future, which registers the fd with the spawner's own
   `EpollReactor` on first poll.
3. The `EpollReactor`'s background thread (owned by, and joined on drop of, the
   `AsyncLinuxSpawner` — not a process-global singleton) blocks on `epoll_wait`,
   and calls the registered `Waker` once the pidfd is readable.
4. `arun`'s minimal `block_on` (in `coreutils_async::block_on` — parks the calling
   thread between polls, no external executor) resumes, and the now-non-blocking
   sync `wait()` retrieves the decoded `ExitStatus`.

## Key decisions

See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs — starting with `0001-native-async-rustils.md`, which explains why this
repository exists ahead of a named consumer and what it deliberately does not build.

## Non-goals

- Not a general-purpose async runtime or executor — `reactor-core` and
  `coreutils_async::block_on` exist to make this workspace's own futures resolvable
  without a hidden dependency, not to be depended on by other projects as one.
- Not a Windows/BSD implementation yet — reserved, pending a real consumer.
- Not an fs/net async surface yet — same reason; `process` is the only domain in
  `rustils` itself that is currently *Active* with a real consumer.
- Not a place where API surface exists because it *might* be needed — every trait
  added here should trace to `coreutils-async` (or a future named consumer)
  actually calling it.
