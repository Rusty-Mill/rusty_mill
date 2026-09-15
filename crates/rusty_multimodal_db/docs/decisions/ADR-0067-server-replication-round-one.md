# ADR-0067: Server Replication, Round One — `Request::FetchSnapshot`

- Status: **Accepted as designed and implemented** (2026-09-14 — the
  owner picked option (a)). Proposed and implemented in the same
  session. See `docs/design/SERVER-REPLICATION-DESIGN.md` for the
  full design.
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
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/protocol.rs` (`Request::FetchSnapshot` (34,
  fieldless), `Response::Snapshot { files: Vec<(String, Vec<u8>)> }`
  (23), `ErrorCode::TooLarge` (14), `MAX_SNAPSHOT_BYTES` (8 MiB),
  `PROTOCOL_VERSION` 24 → 25); `src/server/serve.rs`
  (`ConnectionStore::fetch_snapshot` trait method, default
  `Unsupported`; `read_table_files`/`ReadTableFilesError` — the
  reading twin of `copy_table_files`, size-ceiling-checked before any
  byte is read; `TokenClass::Replication`; `ServeOptions::
  {with_replication_token, replication_token}`; the `check()` slot;
  the `handle_connection` gate requiring `class == Replication`
  specifically, refusing every other class including `ReadWrite`; the
  version-25 gate); `src/server/{dog,memory,entity,relation}.rs`
  (`fetch_snapshot()`, reusing each adapter's existing
  `backup_source`); `src/bin/memory_server.rs`
  (`SERVER_AUTH_REPLICATION_TOKEN`); `src/server/client.rs`
  (`SchemaDrivenClient::fetch_snapshot()`); the Python reference
  client (`protocol.py`, `client.py`) and `driver.py` (refusal-path
  exercise only — see below). `SERVER-001` v0.55.0 / FR-067.
- **One real correction, found during implementation, before the
  Decision above's sketch was fully wired — narrowing nothing about
  the security posture, but load-bearing for it:**
  `ServeOptions::is_configured()` originally checked only
  `read_only_token`/`read_write_token`/certificate classes — not
  `replication_token`. A server configured with *only*
  `SERVER_AUTH_REPLICATION_TOKEN` set would then answer
  `is_configured() == false`, so every connection would start
  already-authenticated at `ReadWrite` (`AUTH-FR-007`'s
  no-tokens-configured default) and `Authenticate` would become a
  no-op — meaning `FetchSnapshot` could never succeed at all (`ReadWrite`
  never satisfies `TokenClass::Replication`) even with a correctly
  presented replication token, and worse, the *rest* of the API would
  sit wide open with no auth gate the operator plainly intended to
  turn on. Caught immediately by
  `tests/server_replication_integration.rs`'s own success-path test.
  Fixed: `replication_token.is_some()` now also flips
  `is_configured()` true — the identical "any single credential
  configured closes the anonymous default" posture a
  `read_only_token`-only server already has. Pinned by two new unit
  tests (`is_configured_is_true_with_only_a_replication_token`,
  `check_classes_a_replication_token_distinctly`).
- Proven: `tests/server_replication_integration.rs` (5 tests, real
  socket, `Memory` domain): a full snapshot written back to a fresh
  directory and reopened via `open_memory_production_stack_portable`
  with the identical record set (the flagship correctness proof, the
  same shape `ADR-0065`'s own backup test established); no configured
  replication token, a `ReadWrite` token, and a `ReadOnly` token all
  refused `Unauthorized` — the acceptance criterion this whole round
  exists to satisfy, that `ADR-0065`'s declined option (c) stays
  closed unless a *separate* credential is configured; a table padded
  past `MAX_SNAPSHOT_BYTES` refused `TooLarge` with nothing streamed.
  `tests/server_python_client.rs` extended with the refusal-path
  assertion at both the current protocol version (`Unauthorized`, no
  replication token configured on that server) and a hand-negotiated
  10 (`unsupported`, client-side, rule 4). `tests/server_client_only.rs`'s
  pinned `Hello` wire example updated 24 → 25.
  `RMDB_REGENERATE_VECTORS=1 cargo test --features client --lib
  protocol::` regenerated `tests/fixtures/wire-vectors.txt`;
  `clients/python/tests/test_vectors.py` (reads the same fixture,
  no hand-duplicated vectors) — 146 subtests green, confirming the
  Python codec's generic `("vec", "u8")` spec walker needed no new
  primitive, only a `__post_init__` converting the decoded `List[int]`
  to real `bytes`. `cargo fmt -p rusty_multimodal_db -- --check` clean;
  `cargo clippy -p rusty_multimodal_db --all-features -- -D warnings`
  clean (lib, bins, and tests, bench excluded per this repo's
  established Windows/Linux-only-bench convention); `cargo test -p
  rusty_multimodal_db --all-features --no-fail-fast` 529 lib tests
  (527 + 2 new) plus every integration target green, including the
  new `server_replication_integration` target.
