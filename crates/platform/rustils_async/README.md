# rustils_async

A native-async sibling to [`rustils`](https://github.com/baileyrd/rustils),
built to satisfy [`rusty_foundation_akb`](https://github.com/Rusty-Mill/rusty_foundation_akb)'s
requirement that platform crates support async and multithreading.

## What this is, and isn't

This is **not** a fork of `rustils`. It depends on `rustils`' `platform`,
`platform-mock`, and `platform-linux` crates (pinned git dependencies — see
the root `Cargo.toml`) for their data types (`Command`, `ExitStatus`,
`PlatformError`, …) and, where sound, their existing sync implementations.
It adds an async trait surface and a real async wait path alongside them,
rather than duplicating rustils' soundness-critical spawn/fork internals.

## Governance this workspace answers to

- **rustils' own `docs/rfc-v2.md`** — the layering (`api/`/`sys/`/`ffi/`),
  object-safe instance traits, `OsStr`-only boundary, decoded `ExitStatus`,
  and RAII/ownership conventions this workspace mirrors. Notably, rustils'
  RFC v2 §3 ("consumer gate") and §5.6 (reactor is explicitly *not* to be
  designed speculatively, pending `rush`) argue against this repo existing
  yet. `docs/adr/0001-native-async-rustils.md` records that as a deliberate,
  acknowledged exception rather than an oversight.
- **Rusty-Mill Foundation AKB** (`rusty_foundation_akb`) — in particular
  [ADR-0160](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/adr/0160-async-io-lifecycle-is-a-provider-framework-not-a-universal-capability.md)
  ("async I/O lifecycle is a provider framework, not a universal
  capability"), `RM-DEV-ASYNC-0001..0003`, and Foundation Principle #4
  ("async-first, sync-complete"). `reactor-core` exists because of this
  document, not instead of it: it is the provider-framework primitives
  layer ADR-0160 calls for, not a blanket "async" trait other crates
  inherit from.
- **Atlas Engineering Standards Library** — `ATLAS-001` (layered
  architecture, RAII, `Result`-over-panic, explicit synchronization
  strategy for shared mutable state) and `ATLAS-600` (branching, commit,
  and PR conventions). Significant, non-obvious decisions are recorded
  under `docs/adr/` per `ATLAS-GOV-ADR-0001`; this repo's one deliberate
  shortcut — starting without a named consumer — is acknowledged per
  `ATLAS-VAL-0011`, not silently taken.

## Workspace layout

| Crate | Role |
|---|---|
| `reactor-core` | Runtime-agnostic async-io primitives (operation identity, cancellation, explicit clock, shutdown). No hidden runtime, no tokio dependency. |
| `platform-async` | Async trait counterparts to `rustils::platform`. Starts with the `process` domain only — the one domain in `rustils` that is already *Active* with a real consumer. |
| `platform-async-mock` | In-memory async backend for consumer tests, mirroring `platform-mock`. |
| `platform-async-linux` | Real Linux backend: reuses `platform-linux`'s spawn, adds a pidfd + epoll async wait path. Windows/BSD are reserved rows below, not stub crates. |
| `threading` | Minimal multithreading primitives (scoped-thread spawn, decoded join outcome, `Mutex`/`RwLock` with an explicit poisoning policy). Deliberately small — the AKB's own threading capability doc is still a draft domain analysis. |
| `coreutils-async` | Reference consumer: `rrun` (rustils' own "reference consumer that gates the process domain's native backends") ported to `arun`, so the API has at least one real caller. |

### Reserved, not built

| Domain | Status | Why |
|---|---|---|
| `platform-async-windows` | Reserved | No forcing consumer yet; IOCP backend follows the same shape as `platform-async-linux` once one exists. |
| `platform-async-bsd` | Reserved | Same. |
| fs / net async domains | Reserved | `process` is rustils' only *Active* domain today; extending async coverage to fs/net follows the same consumer-first reasoning `docs/adr/0001-native-async-rustils.md` argues for everywhere except this repo's own bootstrap. |

## License

MIT, matching every sibling crate in this family (rustils' D-14) so code
can flow in both directions.
