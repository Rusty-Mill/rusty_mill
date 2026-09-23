# Server Replication, Round One: `Request::FetchSnapshot` — a Network-Transferable, Lock-Consistent Full Copy (Implemented)

- Status: **Accepted as designed and implemented** (2026-09-14,
  `ADR-0067`) — implemented in the same session the design was
  proposed. See `ADR-0067`'s own "Acceptance and implementation"
  section for the full implementation record, real corrections found
  along the way, and what's proven.
- Related: `docs/FUTURE-GROWTH.md`'s "Replication/high availability"
  bullet (the gap this round starts closing — explicitly named there
  as "realistically a multi-round effort, not a single ADR"), `ADR-0065`
  (`Backup`, whose lock-consistency mechanism this round reuses, and
  whose own Considered options **already rejected** a wire-streaming
  shape once — see Context below for why this round revives it
  differently), `ADR-0052` (`Compact`, the `with_exclusive` critical-
  section precedent both `Backup` and this round reuse), `ADR-0012`
  (`SERVER-AUTH-DESIGN`, the token-class precedent this round extends),
  `src/server/journal.rs` (the existing local-only crash-recovery
  journal — read and found unsuited to tailing; see Context).

## Purpose and scope

`docs/FUTURE-GROWTH.md` names the gap in full: *"Zero — no
primary/replica concept, no write-ahead shipping beyond the per-adapter
crash-recovery journal (`ADR-0025`/`0026`/`0063`, which never leaves
the one process), no failover. A single process owning a single
directory is the only deployment shape this crate has ever had."* It
also names this as a multi-round effort, unlike `Backup`/`Metrics`/
schema migration, each closed in one round.

This round is the first slice, not the whole thing: **make a full,
lock-consistent, current data set fetchable over the network by a
process that has no filesystem access to the primary's host at all.**
Today `Backup` (`ADR-0065`) produces a lock-consistent copy, but only
onto the *primary's own local disk* (`SERVER_BACKUP_ROOT`) — getting
those bytes to a second machine is left to the operator (`scp`, a
shared network filesystem, ...), which is exactly the "shipping"
`FUTURE-GROWTH.md` names as absent. This round closes that one gap:
a new, separately-gated request streams a full, consistent snapshot
back over the same connection, so a replica process on a different
machine can bootstrap or refresh itself using nothing but network
access to the primary — no shared filesystem, no SSH key, no
operator-run copy step.

## Non-goals

- **Continuous or incremental shipping.** Every fetch is a full
  snapshot, exactly as `Backup` already is. Tailing the batch journal
  or any other incremental/streaming mechanism is a separate, larger
  round — see Context's journal-suitability finding below.
- **Automatic failover or promotion.** Nothing here detects a primary
  outage, elects a new primary, or redirects client traffic. A replica
  refreshed by this round's mechanism is a **cold standby**: promoting
  it means an operator manually pointing new client traffic at it (or
  restarting it as a primary), not anything this crate automates.
- **Write forwarding.** A replica never proxies writes to a primary.
  It is a read-only reopen of a point-in-time snapshot until an
  operator decides otherwise.
- **Cluster membership, gossip, or consensus.** No peer list, no node
  identity, no quorum. Confirmed nothing of this kind exists anywhere
  in `src/` today (a clean slate, not a gap to reconcile with existing
  code).
- **Hot directory swap on a live server.** `SERVER_DATA_DIR` is read
  once at process startup (`ADR-0053`); adopting a freshly fetched
  snapshot means restarting the replica's own `memory_server` process
  pointed at the new directory, not an in-place reopen while serving.
  A zero-downtime hot-swap is real, separate future work.
- **A replica-refresh client/daemon shipped by this crate.** This
  round adds the one new server-side capability (`FetchSnapshot`) and
  a client method to call it; the polling loop, local-disk write, and
  process-restart trigger that turn that into an actual running
  replica are an operator's/consumer's script, not code this crate
  ships — matching `Backup`'s own precedent (`ADR-0065`: "restore
  needs no new code... the produced directory is exactly what
  `open_portable` already reads").

## Context and terminology

Read from `src/server/{serve,journal,protocol,client}.rs`,
`docs/decisions/{ADR-0025,ADR-0026,ADR-0063,ADR-0065}.md`,
`docs/FUTURE-GROWTH.md` as they stand at `SERVER-001` v0.54.0:

- **The server is genuinely single-process, single-directory today.**
  `serve`/`serve_tables` (`src/server/serve.rs`) each take exactly one
  `TcpListener` and one in-process `Arc<S>`/`Vec<Arc<dyn
  ConnectionStore>>` — no peer address, no node id, anywhere in
  `ServeOptions` or either function's signature. `SERVER_DATA_DIR`
  (`src/bin/memory_server.rs`) names one directory for one running
  process; there is no notion of a second process also owning it.
- **`Backup` (`ADR-0065`) is local-disk-only, not a network primitive.**
  `Request::Backup` copies every file a table owns into
  `SERVER_BACKUP_ROOT.join(name)` — a directory on the **same host**
  as the primary. The wire round trip only carries `files`/`bytes`
  counts back to the caller (`Response::BackedUp`), never the bytes
  themselves. A remote replica today needs an operator-run,
  out-of-band transfer of that directory (`scp`, rsync, a shared
  mount) — this crate provides none of that.
- **`Backup`'s own design already considered and rejected a
  wire-streaming shape — read directly, not glossed over.** `ADR-0065`
  Considered options, option (c): *"A wire-level streaming backup
  (bytes returned directly to the client, no server-local write
  target). Removes the path-confinement problem but replaces it with
  a strictly worse one — any authenticated write client could pull an
  entire table's bytes on demand, a new bulk-exfiltration primitive...
  Declined in the design document."* This round revives that same
  wire shape (bytes on the connection) but changes exactly the thing
  that made it unacceptable: it is gated behind a **new, separate,
  opt-in credential** (see Decision) that a `ReadWrite` client does
  not automatically have — `ADR-0065`'s objection was specifically
  that *any* authenticated write client could exfiltrate; this design
  restricts the capability to a distinct token nobody holds unless an
  operator explicitly configures one.
- **The batch journal (`ADR-0025`/`0026`/`0063`) is unsuited to
  tailing/shipping as it stands, on inspection, not assumption.** Its
  on-disk shape (`src/server/journal.rs`) is mechanically the right
  kind of thing to tail — a magic+version header followed by a
  self-describing, length-prefixed `[kind][len][payload]` entry
  stream, no back-references. But three concrete properties rule it
  out for this round: (1) it is truncated aggressively
  (`JOURNAL_CHECKPOINT_BYTES = 1 MiB` triggers a flush-then-truncate
  after almost every meaningfully sized burst — a tailer racing the
  checkpoint could miss entries); (2) truncation is unconditional,
  with no concept of "has a remote consumer acknowledged this entry
  yet"; (3) entries are redo intents (`TransactionOp`/`WriteOp`) that
  only replay correctly against an *already-open store with the exact
  prior state* under its own exclusive lock — a second process would
  need a byte-identical starting snapshot before a single journal
  entry means anything to it, which is precisely the bootstrapping
  problem this round solves, not something journal-tailing could
  bypass. Real journal shipping is a legitimate future round; it needs
  an acknowledgment protocol and a non-destructive retention policy
  neither `ADR-0025`/`0026`/`0063` ever designed for, since all three
  are explicitly, verifiably scoped to local crash recovery only (no
  mention of a second process in any of the three documents).
- **No safe multi-process shared-directory primitive exists.**
  `GENERIC-MMAP-MULTIPROCESS-DIAGNOSIS`/`-APPEND-SLOT-RACE-FIX`
  (`docs/PROJECT-STATUS.md`) proved two processes can safely race on
  *slot creation* in one shared directory (via `O_APPEND`'s
  local-filesystem atomicity guarantee), nothing broader — no general
  read/write coordination, and explicitly not NFS-safe. A replica
  therefore always needs its **own, separate directory** — never a
  directory shared live with the primary's writer process — matching
  what `Backup`/`FetchSnapshot` both already produce.
- **`Compact`/`Backup`'s `with_exclusive` critical section already
  gives lock consistency for free.** Reused unchanged: `FetchSnapshot`
  runs inside the same critical section `Backup` already acquires, so
  the bytes streamed back are exactly as consistent as a `Backup`
  copy — no new coordination primitive.
- **Token classes today: `ReadOnly`/`ReadWrite` only**
  (`src/server/mod.rs`, `ADR-0012`). No precedent yet for a third
  class scoped to one specific, sensitive capability — this round is
  the first.

## Requirements

- `RPL-FR-001` **A new `Request::FetchSnapshot`, answered
  `Response::Snapshot { files: Vec<(String, Vec<u8>)> }`** (or an
  equivalent framing that keeps every file's name and full bytes on
  one response — see Proposed shape for the exact framing question
  this leaves open). Reuses `ConnectionStore`'s existing
  `with_exclusive` critical section exactly as `Backup`/`Compact` do;
  no new locking.
- `RPL-FR-002` **Gated behind a new, distinct token class —
  `Replication` — never satisfied by a `ReadOnly` or `ReadWrite`
  token.** A server with no replication token configured answers every
  `FetchSnapshot` `Unauthorized`, the same "opt-in, zero new surface
  by default" posture `Backup`'s `SERVER_BACKUP_ROOT` already
  established for local disk. Directly answers `ADR-0065`'s own
  rejection of option (c) — see Context.
- `RPL-FR-003` **Reuses `copy_table_files`'s existing file-enumeration
  logic** (the prefix-glob "every file whose name starts with the
  base path's own file name" rule `ADR-0065` already proved correct by
  construction against every companion-file kind this crate has), just
  reading each file's bytes into the response instead of copying it to
  a second local path. No new file-discovery mechanism.
- `RPL-FR-004` **`ConnectionStore::fetch_snapshot` trait method,
  default `Unsupported`** — the same opt-in-per-adapter shape
  `backup`/`compact` already use; `Dog` stays mechanically capable but
  unwired, matching `Backup`'s own precedent for a non-durable binary.
- `RPL-FR-005` **`SchemaDrivenClient::fetch_snapshot(name) -> Result<
  Snapshot, ClientError>` and its Python equivalent** — a real client
  a replica-refresh script can call; ships no daemon or poll loop
  itself (see Non-goals).
- `RPL-FR-006` **A documented replica pattern, not shipped automation.**
  This design doc's own "Proposed shape" names the exact recipe an
  operator's script follows (fetch, write to local disk, restart a
  second `memory_server` pointed at it) — real, concrete, and provably
  correct against this crate's own `open_portable` contract, but not
  code this crate runs on a schedule. *Since `ADR-0118`:* it ships as
  `examples/replica_refresh.rs`, once or on an interval.
- `RPL-FR-007` **A size ceiling, named explicitly, not left
  implicit.** Streaming every file's full bytes in one response departs
  from every prior request's small, bounded payload shape — the
  requirement is a named limit (`MAX_SNAPSHOT_BYTES` or equivalent,
  see Open questions) and a clear, typed refusal
  (`ErrorCode::Malformed`/a new code) when a table's on-disk size
  exceeds it, rather than an unbounded in-memory buffer on either end.

## Considered options

- **(a) `Request::FetchSnapshot`, a new `Replication` token class —
  recommended, as scoped above.** Closes the "shipping beyond one
  process" gap with the smallest new wire surface: one request, one
  response, one token class, full reuse of `Backup`'s locking and
  file-enumeration. Directly answers `ADR-0065`'s own rejection of
  streaming bytes by narrowing who can ever ask.
- **(b) Reuse `Backup` unchanged; document an out-of-band transfer
  step (rsync/scp/shared mount) as the "shipping" answer.** Zero new
  code, zero new attack surface. Con: this is exactly today's status
  quo — `docs/FUTURE-GROWTH.md` already names it as the gap, not a
  closed question; declining again adds no information, the same
  reasoning `ADR-0066`'s own option (c) used to prefer building
  something real over declining a second time.
- **(c) Full replication protocol now: continuous journal shipping,
  replica acknowledgment, automatic promotion.** The complete shape
  `FUTURE-GROWTH.md` describes as the end state. Rejected for this
  round specifically: `FUTURE-GROWTH.md`'s own text calls this
  "realistically a multi-round effort," and Context above shows the
  journal is not shippable as it stands without its own separate
  design (acknowledgment protocol, non-destructive retention) — this
  would not be one bounded round, it would silently reopen `ADR-0025`/
  `0026`/`0063` mid-flight.
- **(d) Decline entirely this round; further design only, no wire
  change.** Matches `GENERIC-SCHEMA-DESIGN`/`SERVER-QUERY-LAYER-
  DESIGN`'s own "stop before any code" precedent for the single
  biggest decisions this crate has faced. Worth naming as a real
  option given replication's own size, but this round's slice (a) is
  small enough — one request, reusing every existing lock/enumeration
  primitive — that treating it at that same maximum-caution tier would
  be disproportionate; `ADR-0065` itself (a comparably novel
  attack-surface category — the first caller-chosen write target) was
  design-then-implement-same-session, not design-only.

The owner's shorthand: **(a)** `FetchSnapshot` + new token class, as
proposed; **(b)** decline, document the manual transfer step only;
**(c)** the full multi-round protocol now instead of a bounded first
slice; **(d)** design-only this round, implementation deferred to a
separate owner check-in.

## Proposed shape

`src/server/protocol.rs`: `Request::FetchSnapshot { name: String }`
(reusing `Backup`'s own single-path-component validation for `name` —
though note `FetchSnapshot` never writes to a server-local path itself,
so `name` here is closer to a request label than a path-confinement
target; Open questions asks whether `name` is even needed, or whether
`FetchSnapshot` should simply always answer for "the whole table, no
name"), `Response::Snapshot { files: Vec<(String, Vec<u8>)> }` (or a
multi-frame streaming shape — see Open questions), `PROTOCOL_VERSION`
24 → 25.

> **As implemented** (see Open questions — resolved): `Request::FetchSnapshot`
> is fieldless (no `name`); the dispatch arm needs no `handle_fetch_snapshot`
> special-case — unlike `Backup`, it needs no `ServeOptions` access
> beyond the `Replication`-class check already performed earlier in
> `handle_connection`, so it goes through the generic `dispatch` loop.

`src/server/serve.rs`: `ConnectionStore::fetch_snapshot(&self) ->
Result<Vec<(String, Vec<u8>)>, ErrorCode>` (default `Unsupported`);
a `handle_fetch_snapshot` dispatch arm gated on the new `Replication`
token class and a version ≥ 25 check, refusing `Malformed` below it
(the `Backup`/`Compact` gating precedent exactly); reuses
`copy_table_files`'s file-list logic, reading bytes via `std::fs::read`
per file instead of copying.

`src/server/mod.rs`: a third `TokenClass` variant, `Replication` — or,
if the owner prefers not to grow the existing enum, a separate
`AuthConfig` field (`replication_token: Option<String>`) checked
independently of `ReadOnly`/`ReadWrite` — named as an explicit
open question below since it changes an already-`Accepted`,
multiply-implemented enum (`ADR-0012`).

`src/server/client.rs`: `SchemaDrivenClient::fetch_snapshot(name) ->
Result<Snapshot, ClientError>`; `Snapshot { files: Vec<(String,
Vec<u8>)> }` (or equivalent) as a new public type.

An operator's replica-refresh recipe this round makes possible, real
and concrete but **not shipped as running code** (*shipped since
`ADR-0118` as `examples/replica_refresh.rs`*):

```text
1. connect(replica_credentials_with_replication_token)
2. snapshot = client.fetch_snapshot(table_name)
3. write every (name, bytes) pair to a fresh local directory
4. stop the replica's own memory_server (if running)
5. point SERVER_DATA_DIR at the fresh directory, start memory_server
6. repeat on whatever schedule freshness requires
```

Step 3's write must itself be crash-safe (temp-then-rename), matching
this crate's own established discipline throughout — named here so a
reference implementation of the operator script (if one is ever
written, e.g. as a second `examples/` entry in a follow-up round) does
not silently skip it.

## Data/state and invariants

- Every fetched file is read from disk *after* the file enumeration
  and lock acquisition `Backup` already proves gives a live-consistent
  set — no new consistency argument needed.
- The response payload is bounded by `RPL-FR-007`'s named ceiling; a
  table whose on-disk size exceeds it is refused, not silently
  truncated or partially streamed.
- No file is ever written by the primary process as a side effect of
  `FetchSnapshot` — unlike `Backup`, which writes to
  `SERVER_BACKUP_ROOT`, this request only reads.

## Errors, failure, recovery, and observability

- No `Replication` token configured: every `FetchSnapshot` answers
  `Unauthorized`, server-wide — zero new surface by default, exactly
  `Backup`'s unset-`SERVER_BACKUP_ROOT` posture.
- A wrong/missing `Replication` token on an otherwise-valid connection:
  `Unauthorized`, the same class `ReadOnly`/`ReadWrite` already use for
  a capability check that fails.
- Below protocol version 25: `Malformed`, the version-gate precedent
  every prior write-shaped request already follows.
- A table's on-disk size over the named ceiling: refused with a typed
  error before any file is read into memory (not discovered
  mid-transfer).
- No file, ever, is left partially written on the *primary* side (it
  never writes at all). A replica-side partial write is the
  operator script's own responsibility, named in Proposed shape.

## Security, privacy, and compatibility

- **The real, named new attack surface**: an authenticated
  `Replication`-token client can pull an entire table's bytes in one
  request — genuinely a bulk-exfiltration-shaped capability, the exact
  risk `ADR-0065`'s option (c) named. Mitigated by (1) a token class
  distinct from `ReadOnly`/`ReadWrite` that nobody holds unless an
  operator explicitly provisions one (unlike `ReadWrite`, which a
  real deployment may hand to several ordinary write clients already);
  (2) unset by default, server-wide `Unauthorized` with no
  configuration; (3) TLS (`ADR-0014`/`ADR-0023`) already available and
  strongly recommended for this token exactly as it already is for
  `ReadWrite` — named here, not newly invented.
- Wire, append-only, hard-to-reverse-once-shipped: `PROTOCOL_VERSION`
  bump, new request/response variants, a new token-class surface —
  exactly the class of decision `WORKFLOW.md` requires design-first,
  owner-accepted before implementation.

## Acceptance criteria

1. No `Replication` token configured: every `FetchSnapshot` answers
   `Unauthorized` before any file is touched. **Proven:**
   `fetch_snapshot_is_unauthorized_with_no_configured_replication_token`.
2. A `ReadWrite`-only token (no `Replication` token presented):
   `FetchSnapshot` answers `Unauthorized` — proves `ReadWrite` alone
   never grants this capability, directly closing `ADR-0065`'s own
   named objection to option (c). **Proven:**
   `fetch_snapshot_is_unauthorized_for_a_read_write_token` (and, for
   completeness, `..._for_a_read_only_token`).
3. A valid `Replication` token: `FetchSnapshot` returns every file the
   table owns, byte-for-byte identical to what `Backup` would have
   copied to local disk in the same instant (proven by comparing
   against a `Backup` taken back to back, or by writing the fetched
   bytes to disk and reopening via `open_..._production_stack_portable`
   with the identical record set — the same flagship proof `ADR-0065`
   used). **Proven:**
   `fetch_snapshot_produces_a_reopenable_directory_with_the_identical_records`.
4. A table exceeding the named size ceiling is refused before any
   bytes are read, not partially streamed. **Proven:**
   `fetch_snapshot_refuses_a_table_over_the_size_ceiling`.
5. `FetchSnapshot` below protocol version 25 is `Malformed`. **Proven:**
   the `Request::FetchSnapshot if negotiated < 25` gate in
   `handle_connection`, exercised by `driver.py`'s hand-negotiated
   version-10 exercise in `tests/server_python_client.rs`
   (`fetch_snapshot` answers `unsupported` client-side, rule 4).
6. `Dog` is mechanically capable (`fetch_snapshot` implemented where
   `TransactionalStore` already gives exclusive access) but no shipped
   binary wires a `Replication` token to it, matching `Backup`'s own
   precedent for a non-durable binary. **Implemented as designed:**
   `DogConnectionStore::fetch_snapshot` exists; `dog_server.rs` wires
   no `SERVER_AUTH_REPLICATION_TOKEN` (it has no `SERVER_BACKUP_ROOT`
   either — the same precedent, unchanged by this round).

## Verification plan

`cargo test --all-features` (per-adapter unit tests for
`fetch_snapshot`; a new `tests/server_replication_integration.rs`, a
real-socket test seeding data, fetching a snapshot, writing it to a
fresh local directory, and reopening it via the real
`open_..._production_stack_portable` with the identical record set —
the same flagship shape `tests/server_backup_integration.rs` already
established); `cargo fmt`/`cargo clippy --all-features -- -D warnings`
clean.

## Traceability

- Roadmap: `SERVER-REPLICATION-DESIGN` (this document, `Implemented`),
  `SERVER-REPLICATION` (implementation, `Implemented`).
- `docs/FUTURE-GROWTH.md`'s "Replication/high availability" bullet
  updated to the "Partly built since this was written" treatment
  `Backup`/`Metrics`/schema migration each received — explicitly
  still absent: continuous shipping, failover, promotion, write
  forwarding (see Non-goals, all unchanged by this round).

## Open questions — resolved during implementation

- **Does `FetchSnapshot` need a `name` field at all?** **Resolved: no.**
  Implemented exactly as the recommendation above — `Request::FetchSnapshot`
  is fieldless; it answers for "this table, now," with no caller-chosen
  string at all.
- **`TokenClass::Replication` as a third enum variant, or a separate
  `AuthConfig` field?** **Resolved: the enum variant**, exactly as
  recommended — `TokenClass::Replication` is a third
  `#[derive(..., PartialEq, Eq)]` variant, so every existing `match`
  on `TokenClass` (`ServeOptions::check`, the `ReadOnly`-blocks-writes
  gate, `Debug`) is compiler-forced to account for it.
- **The exact wire framing for a potentially large, multi-file
  payload?** **Resolved: one `Response::Snapshot` with every file's
  bytes inline**, exactly as recommended — no multi-frame/chunked
  shape this round; `MAX_SNAPSHOT_BYTES` (8 MiB) sits comfortably
  under `MAX_FRAME_BYTES` (16 MiB), so a snapshot at the ceiling
  always fits in one frame.
- **Should `MAX_SNAPSHOT_BYTES` be operator-configurable?**
  **Resolved: no — a fixed constant this round**, exactly as
  recommended, matching `MAX_BATCH_OPS`'s own "nobody has asked for a
  different number yet" precedent.

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-14: the owner picked option (a); implemented the same
  session. See `ADR-0067`'s own "Acceptance and implementation"
  section for the full record.
