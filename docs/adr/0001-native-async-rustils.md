# ADR-0001: Build native async support for rustils ahead of a named consumer

**Status:** Accepted
**Date:** 2026-08-12

## Context

`rusty_foundation_akb` requires every platform crate in the Rusty-Mill
ecosystem to support async and multithreading. `rustils` has neither
today: every trait in `platform` is synchronous and blocking, and its own
governing document, `docs/rfc-v2.md`, is explicit about why:

- §3, the **consumer gate**: "no API is implemented without a named,
  working consumer that calls it." This is framed as the structural
  defense against the "expansion-by-conversation" dynamic that produced
  the project's original, abandoned scaffold.
- §5.6: the reactor (wait-any over children/handles/events with a
  timeout) — the exact machinery async waiting needs — is explicitly
  **contracted, not to be designed speculatively**. It is scoped to
  arrive from a sibling project, `rush`, "with its semantics already
  proven."

Separately, `rusty_foundation_akb` itself, in
[ADR-0160](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/adr/0160-async-io-lifecycle-is-a-provider-framework-not-a-universal-capability.md),
already rejected the idea of one universal async capability other crates
inherit from — it commits instead to a shared *provider framework* of
lifecycle/safety primitives (cancellation, explicit clock, wake/executor
adapters, shutdown, generation-scoped operation identity), with domain
semantics staying in each domain capability.

Today there is no named, working consumer forcing `rustils` toward async.
`rusty_foundation_akb` itself is still spec-only (no code). Building this
now is a deliberate exception to rustils' own consumer gate, made and
recorded explicitly rather than silently, per Atlas `ATLAS-VAL-0011`
("architectural shortcuts MUST be explicitly acknowledged") and
`ATLAS-GOV-ADR-0001` (significant, non-obvious decisions recorded as an
ADR when made).

## Decision

Build `rustils_async` now, as a separate repository, structured to
minimize how much speculative surface it actually contains:

1. **Primitives only in `reactor-core`**, matching ADR-0160's provider-
   framework shape: operation identity, cancellation, an explicit clock,
   shutdown signaling. No domain semantics live here.
2. **One domain to start: `process`.** It is the one domain in `rustils`
   already marked *Active* with a real consumer (`coreutils`). The async
   value-add is scoped to `Child::wait` only (RM-DEV-ASYNC-0001: async is
   for genuine waiting/multiplexing, not for spawn itself, which is a
   single fast syscall). `AsyncSpawner::spawn` calls straight through to
   `rustils`' own `platform-linux::Spawner::spawn`, synchronously —
   this workspace does not re-implement fork/exec, so it does not
   reproduce the soundness risk rustils' own RFC v2 §6 (B-1..B-5) spent
   real effort closing.
3. **A real forcing consumer is still named**, even though the gate is
   being bypassed: `coreutils-async` ports `rrun` — rustils' own
   "reference consumer that gates the process domain's native
   backends" (its doc comment's words, not this ADR's) — to `arun`, so
   the trait shapes have at least one caller exercising them rather than
   being purely speculative. `rcat` was considered first and rejected:
   it exercises the `fs` domain, which this workspace does not build an
   async surface for (see point 2), so it would not have exercised
   anything this repo actually adds.
4. **Windows/BSD backends and the fs/net async domains are reserved as
   table rows** in the README, not built as empty stub crates — the same
   "row retained, stub deleted" convention rustils' own RFC v2 §3 uses
   for its parked domains.
5. **`threading` stays minimal**, scoped to what the AKB's own threading
   capability doc already treats as settled (thread lifecycle, mutex/
   rwlock with an explicit poisoning policy per Atlas `ATLAS-STATE-0001`)
   and skipping what that doc itself still lists as draft (wait
   primitives, atomics, scheduling/affinity) — building those now would
   be exactly the speculative-abstraction problem Atlas's Economy value
   (`ATLAS-NONGOAL-0030`/`0031`) exists to prevent.

## Consequences

- This repository's existence, and its reactor-adjacent code in
  particular, is out of step with rustils' own RFC v2 §3/§5.6 until one
  of two things happens: `rustils_async` itself becomes the named
  consumer that RFC is amended to recognize, or `rush`'s proven reactor
  shape arrives and this crate is reconciled against it (possibly
  replaced by it).
- `reactor-core`'s primitives are intentionally not extracted into the
  separate `rusty_async` repository yet. Atlas `ATLAS-DEP-0010` requires
  a concrete forcing function before splitting a service/repository out
  of a modular monolith — a second real consumer beyond this repo is
  that forcing function, and none exists yet.
- Every future expansion of this workspace's surface (new domains, new OS
  backends) should still go through the same test this ADR applies:
  is there a real caller, even a small one, or is it speculative?

## Alternatives rejected

- **Wait for `rush`'s reactor to hoist**, per rustils' RFC v2 §5.6/§7, and
  do nothing here until then. Rejected because `rusty_foundation_akb`'s
  requirement is current, not scheduled against `rush`'s own phase gates,
  which this repository does not control.
- **Build this inside the `rustils` repository/workspace directly**,
  which is what Atlas `ATLAS-DEP-0010`'s modular-monolith default would
  normally argue for. Rejected for this iteration because rustils' own
  RFC v2 §5.6 explicitly reserves the reactor's module names and design
  for `rush`; adding a competing async design inside that same workspace
  risks colliding with that reservation more directly than a sibling
  repository does. This is recorded as a tradeoff, not a settled
  position — revisit if `rustils_async` proves out and a merge back
  becomes the better shape.
- **Depend on `tokio` (or another runtime) directly** in `reactor-core`.
  Rejected: ADR-0160 (`RM-ASYNC-RUNTIME-0001`) requires that an I/O engine
  not require one global executor or create a hidden runtime;
  `reactor-core` uses only `std::task::Waker` and its own `Clock`
  abstraction so the same primitives work under any executor.
- **Use the `async-trait` crate** for object-safe async traits. Rejected
  in favor of hand-written `Pin<Box<dyn Future<...> + Send>>` return
  types — mechanically what the macro would generate, without adding a
  dependency, matching rustils' own minimal-dependency discipline.
