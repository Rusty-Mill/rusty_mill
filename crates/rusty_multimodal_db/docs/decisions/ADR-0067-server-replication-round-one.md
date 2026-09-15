# ADR-0067: Server Replication, Round One — `Request::FetchSnapshot`

- Status: **Proposed** (2026-09-14), design only. See
  `docs/design/SERVER-REPLICATION-DESIGN.md` for the full design.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md`'s "Replication/high availability"
  bullet (names this a multi-round effort — this ADR is round one, not
  the whole thing), `ADR-0065` (`Backup`, whose lock-consistency and
  file-enumeration mechanism this round reuses, and whose own
  Considered options already rejected a wire-streaming shape once —
  see the design doc's Context for why this round revives it safely),
  `ADR-0012` (the `TokenClass` precedent this round extends to a
  third class), `ADR-0025`/`ADR-0026`/`ADR-0063` (the batch journal —
  read and found unsuited to tailing/shipping as it stands).
- Supersedes/Superseded by: none proposed.

## Context

`docs/FUTURE-GROWTH.md` names the gap: *"Zero — no primary/replica
concept, no write-ahead shipping beyond the per-adapter crash-recovery
journal ... which never leaves the one process, no failover."* It also
names this, unlike every prior operational-maturity item, as
"realistically a multi-round effort, not a single ADR."

`Backup` (`ADR-0065`) already produces a lock-consistent copy, but only
onto the *primary's own local disk* — getting those bytes to a second
machine is left entirely to the operator today. That is the one
concrete, bounded gap this round closes: make a full, consistent
snapshot fetchable over the network by a process with no filesystem
access to the primary's host at all.

`ADR-0065`'s own Considered options already tried this shape once —
option (c), "a wire-level streaming backup," declined because "any
authenticated write client could pull an entire table's bytes on
demand, a new bulk-exfiltration primitive." This round revives the
same wire shape but changes exactly what made it unacceptable: it is
gated behind a new, distinct `Replication` token class that a
`ReadWrite` client does not automatically hold.

## Decision

**Recommended: option (a)** — a new `Request::FetchSnapshot`, answered
with every file a table owns, gated behind a new `Replication` token
class (never satisfied by `ReadOnly`/`ReadWrite`), reusing `Backup`'s
`with_exclusive` lock and `copy_table_files`'s file-enumeration
unchanged. `PROTOCOL_VERSION` 24 → 25. A named size ceiling
(`MAX_SNAPSHOT_BYTES`) refuses an over-large table rather than
streaming or buffering unboundedly. No replica-refresh daemon or
polling loop ships in this crate — the design doc's own "Proposed
shape" names the exact operator recipe, matching `Backup`'s own
"restore needs no new code" precedent.

Full reasoning, the three alternatives (reuse `Backup` and document a
manual transfer step; build the full multi-round replication protocol
now; decline/design-only this round), and every requirement/acceptance
criterion are in `docs/design/SERVER-REPLICATION-DESIGN.md`.

## Consequences

- Positive: closes the one concrete, bounded piece of
  `FUTURE-GROWTH.md`'s named gap — "shipping beyond one process" — with
  the smallest new wire surface: one request, one response, one token
  class, zero new locking or file-discovery mechanism.
- Named, not hidden: this reopens a shape `ADR-0065` explicitly
  declined once. The design doc's Context section states the prior
  rejection verbatim and names precisely what changed (a separate,
  opt-in credential, not `ReadWrite` itself) — not silently
  reintroduced as if it were new.
- Named, not hidden: a real, if narrowed, bulk-exfiltration-shaped
  capability exists once a `Replication` token is configured. Same
  mitigation posture as `Backup`'s own novel-attack-surface admission —
  unset by default, TLS strongly recommended, a distinct credential
  nobody holds unless an operator explicitly provisions one.
- Explicitly, this round does **not** deliver replication or high
  availability by itself — no continuous shipping, no automatic
  failover or promotion, no write forwarding, no cluster membership.
  It delivers the one missing primitive a real (manually operated)
  cold-standby replica needs and does not have today.
- Wire, append-only, hard-to-reverse-once-shipped — the class of
  decision `WORKFLOW.md` requires design-first, owner-accepted before
  implementation.

## Considered options

**(a) `Request::FetchSnapshot` + a new `Replication` token class** —
recommended; **(b)** reuse `Backup` unchanged, document a manual
out-of-band transfer step only; **(c)** build the full multi-round
replication protocol (continuous journal shipping, acknowledgment,
automatic promotion) now instead of a bounded first slice; **(d)**
decline entirely this round, design-only, no wire change.

The owner's shorthand: **(a)** as proposed; **(b)** decline, document
only; **(c)** the full protocol now; **(d)** design-only, defer
implementation to a separate check-in.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
