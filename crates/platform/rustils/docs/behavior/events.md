# Behavior Spec — events (SignalSource)

**Capability identity & maturity** (#114): `rustils.events.signal_source`
— Trial (Linux installation/delivery fully asserted in CI; Windows pins
installation and empty-slot semantics only — delivery isn't behaviorally
asserted, see Deliberately unspecified below). See `docs/rfc-v2.md` §10
for the numbered decision log.

The deferred-signal core (extraction map D6, from rush's `trap.rs`),
asserted against `platform-linux`, `platform-windows`, and the mock.

## Specified

- The OS-level handler performs exactly one atomic store — no
  allocation, locks, I/O, or callbacks. Everything observable happens at
  the consumer's safe points via `take()`.
- `take()` never blocks; it consumes (a second `take` with no new
  delivery returns `None`).
- The slot is single-entry: deliveries between two `take()` calls
  coalesce, keeping the most recent event. Consumers that poll around
  `wait_any` ticks observe every *quiet-period* event but not every
  event of a burst — this is the donor mechanism's documented shape, not
  a defect to fix silently.
- `install` is process-global (signal disposition is per-process state
  on every OS) and idempotent.
- Identity mapping: `Interrupt` = SIGINT / `CTRL_C_EVENT`; `Terminate` =
  SIGTERM / `CTRL_BREAK_EVENT`; `Hangup` = SIGHUP / `CTRL_CLOSE_EVENT`
  (divergence 003).
- Deferral means survival: a process with the source installed is not
  terminated by the default disposition of a mapped signal; it observes
  the event at its next safe point instead. (Pinned by the Linux parity
  test: the test process outlives a real SIGTERM.)

## Async path (rusty_foundation_akb §9)

`SignalSource::take` never blocks by design (Specified above) — it reads
a single-entry coalescing slot and returns immediately whether or not an
event is pending. This trivially satisfies §9 rule 1 ("MUST NOT occupy a
worker thread solely while waiting"): there is no waiting state to occupy
a thread with. A consumer that wants to react promptly polls `take` at
its own safe points (or on a timer); `platform` defines no push/callback
API here, since D6's whole design is deferred delivery to a safe point,
not immediate async notification.

**Cancellation safety:** trivial — `take` has no in-flight state to
cancel. **Completion ownership:** the caller owns the returned
`SignalEvent` outright. **Backpressure:** the single-entry slot *is* the
backpressure mechanism — a second event of the same kind before the first
is drained coalesces rather than queuing (Specified above).

## Deliberately unspecified / not asserted

- **Windows delivery is not behaviorally asserted in CI**: runners have
  no interactive console, and `GenerateConsoleCtrlEvent` addresses
  console process groups the test harness does not control. The Windows
  leg pins installation and empty-slot semantics only, with the
  reasoning recorded in the test (D7 discipline).
- Ordering within a burst (single slot — see above).
- The full unix signal set: consumers with policy needs (rush's `trap`)
  extend at the policy layer; this mechanism ships the portable three.
