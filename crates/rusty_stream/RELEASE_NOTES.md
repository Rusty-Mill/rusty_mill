# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

No version tags yet (pre-1.0, nothing published). Tracked by unit of change,
reverse chronological, each linking to its PR once one exists.

---

## A Dockerfile for the standalone server
**2026-08-02**

- **Added:** `Dockerfile` (multi-stage: full Rust image to build, minimal
  `debian:bookworm-slim` to ship just the release binary) and
  `.dockerignore`. `RUSTY_STREAM_ADDR` defaults to `0.0.0.0:7420` inside
  the image (not `127.0.0.1`, so other containers can reach it);
  `RUSTY_STREAM_DATA_DIR` defaults to `/data`, meant to be mounted as a
  volume. Written for `baileyrd/deft-data-sharing-sample`'s
  `docker-compose.yml`, which builds against this image via a git build
  context.
- **Known limitation, stated plainly:** not verified with a real `docker
  build`/`docker run` — no Docker daemon was available in the environment
  that wrote it. Verify before relying on it for a real deployment.

## A real standalone server binary
**2026-08-02**

- **Added:** `src/main.rs` — `rusty_stream` is now something you can
  actually run, not just a library exercised by tests. Wires
  `rusty_tokio::io::uring_global_driver()` (the real, production
  `Arc<dyn OpDriver>` — not `SimDriver`) to a real `Log`/`ConsumerOffsets`
  and `server::serve` over a real bound `TcpListener`. Configuration is
  two environment variables (`RUSTY_STREAM_ADDR`, `RUSTY_STREAM_DATA_DIR`),
  both optional with sane defaults. Recovers an existing log/consumer
  offsets under `RUSTY_STREAM_DATA_DIR` on startup if one exists (falling
  back to creating fresh ones), and `Ctrl-C` triggers `serve`'s graceful
  shutdown rather than a hard kill.
- **Changed:** bumped the pinned `rusty_tokio` rev to pick up
  [`baileyrd/rusty_tokio#256`](https://github.com/baileyrd/rusty_tokio/issues/256)'s
  fix (`uring_global_driver` made public) — this is the whole reason a
  standalone binary is possible now. Every existing test still passes
  unmodified against the bumped rev.
- **Verified manually, end to end, against the real binary** (not just
  `cargo test`): started the server, ran the sample `deft-data-sharing-sample`
  repo's real `app-a`/`app-b`/`app-c` binaries against it over a real
  socket (produce, two independent consumers each seeing all 5 events),
  sent `SIGINT` and confirmed the graceful-shutdown log line and a clean
  process exit, then restarted against the same data directory and
  confirmed both the log's records and each consumer's committed progress
  survived — a real crash-recovery proof, not just the `SimDriver`-backed
  version `retention.rs`'s/`consumer.rs`'s own tests already covered.
- **Known limitations, stated plainly:** retention policy
  (`max_segment_bytes: 128 MiB`, no size/age limits) is a fixed default,
  not yet configurable via environment variable or config file. No
  structured logging (plain `println!`). No metrics/health endpoint.

## Graceful shutdown on the server
**2026-08-02**

- **Changed:** `server::serve`'s signature gained a
  `shutdown: rusty_tokio::sync::watch::Receiver<bool>` parameter. Its caller
  holds the paired `Sender`; sending `true` stops the accept loop from
  taking any *new* connection while every connection already in flight —
  tracked in a `rusty_tokio::task::JoinSet` rather than a plain
  `Vec<JoinHandle<_>>` — keeps running until it finishes on its own (the
  peer disconnects, or a real I/O error). `serve` itself returns `Ok(())`
  only once the last one has drained; nothing in flight is aborted just
  because shutdown was requested. Built on `rusty_tokio::select!` (2 to 5
  branches, no `if` guards) racing `listener.accept()` against
  `shutdown.changed()`.
- 1 new test (67 total): a connection open before shutdown is requested
  keeps serving requests normally after the request, and only finishes
  (letting `serve` itself return, no `.abort()` needed) once the client
  actually disconnects.
- **Known limitations, stated plainly:** this is the last of the three
  gaps this file has been tracking since the socket-integration entry
  above (consumer-offset wire exposure, frame-size cap, and now graceful
  shutdown are all closed). There's still no bound on how long a slow or
  stuck connection can hold up a drain — a real deployment wanting a hard
  shutdown deadline would need to layer a timeout on top of awaiting
  `serve` itself, not something this pass adds.

## Frame-size sanity cap on the server
**2026-08-02**

- **Added:** `server::MAX_FRAME_LEN` (16 MiB) — `handle_connection` checks
  a frame's declared body length against it right after reading the 4-byte
  header, before allocating a buffer or reading a single byte of the body.
  A client claiming a longer body gets its connection ended immediately
  with an `InvalidData` error instead of the server allocating (and then
  waiting on) whatever the client claimed.
- 2 new tests (66 total): a frame one byte over the cap ends the connection
  before the (in the test, never sent) body is read, and a frame whose
  encoded length lands exactly on the cap — not just comfortably under it —
  still round-trips normally.
- **Known limitations, stated plainly:** still no graceful shutdown on the
  server (unchanged by this PR). `MAX_FRAME_LEN` is a fixed constant, not
  yet configurable per deployment.

## Manifest persistence, closing `retention.rs`'s two documented recovery gaps
**2026-08-02**

- **Added:** `src/manifest.rs`'s `Manifest` — a dedicated `Segment` of
  `Opened`/`Deleted` events, replayed on `Manifest::open_on` to reconstruct
  which segments currently exist and their real creation times. Same
  reused-`Segment` pattern `ConsumerOffsets` already uses, rather than a
  second hand-rolled recovery path. Every event is synced immediately
  (unlike a regular record append) — segment lifecycle changes are rare
  structural events, not the hot per-record path fsync policy exists to
  make configurable.
- **Changed:** `retention::Log::open` no longer takes an
  `existing_base_offsets: &[Offset]` parameter — it reads the manifest
  instead. `Log::create` now also creates a manifest and records its
  initial segment. `roll()` records a new segment's `Opened` event right
  after creating its file (not before); `delete_oldest_closed` records a
  `Deleted` event before actually removing the file, not after — both
  orderings chosen so a crash between the manifest write and the file
  operation leaves only a harmless orphan file, never a manifest entry
  pointing at a segment that isn't there.
- **Fixed:** recovered closed segments now carry their real creation time
  (from the manifest), not the moment `Log::open` happened to run — the
  second gap `retention.rs`'s docs previously called out. Time-based
  retention is now accurate across a restart, not just within one
  process's uptime.
- 7 new tests (64 total): 4 in `manifest.rs` (empty manifest, ordered
  replay, a deleted segment drops out of the live list, a torn event is
  truncated away not served) and 3 in `retention.rs` (the existing
  recovery test updated for the new `Log::open` signature, a deleted
  segment stays deleted across a restart, and a recovered segment's age is
  real rather than reset at open).
- **Known limitations, stated plainly:** a crash between deleting a
  segment's manifest entry and removing its file leaves an orphan file on
  disk — wasted space, cleaned up manually, never a correctness problem.
  Still no graceful shutdown or frame-size cap on the server (unchanged by
  this PR).

## Client SDK: the last item on Phase 1's scope list
**2026-08-02**

- **Added:** `src/client.rs`'s `Client` — `docs/phase1-scope.md` §2's
  "Client SDK — Rust first," implemented last of the six Phase 1 scope
  items. `Client::connect` opens one `rusty_tokio::io::TcpStream`;
  `produce`/`fetch`/`commit`/`last_committed` each encode a `Request`, frame
  it, write it, and decode the framed `Response` back — the same
  hand-framing `server.rs`'s own tests previously had to do inline, now a
  real reusable API. A `Response::Error` from the server surfaces as
  `ClientError::Server`, not a panic or a silently dropped message.
- 5 new tests (58 total), all against a real spawned `server::serve` over a
  real loopback socket: produce-then-fetch and commit-then-last-committed
  round trips through the client API, an unknown consumer reads back `None`
  not an error, fetching an unknown offset comes back as
  `ClientError::Server`, and two independent clients against the same
  server stay independent.
- **Known limitations, stated plainly:** one `Client` wraps one connection
  and is not safe to call from two tasks concurrently — nothing multiplexes
  responses back to a specific in-flight request, so concurrent use means
  one `Client` per task (or a lock around it), the same tradeoff
  `server::AppState` makes explicit on the other side. No connection
  pooling, retry, or reconnect logic — a dropped connection is a dropped
  connection.

## Wire exposure for consumer-offset commits
**2026-08-02**

- **Added:** `protocol::Request::Commit { consumer_id, offset }` and
  `Request::LastCommitted { consumer_id }`, with matching
  `Response::Committed` and `Response::LastCommitted { offset:
  Option<Offset> }` — the write/read pair for consumer offsets that
  `Produce`/`Fetch` already are for the log itself. `LastCommitted` for a
  consumer that's never committed returns `offset: None`, not an error — a
  fresh consumer is expected, not exceptional. New `write_str`/`read_str`
  helpers encode a consumer ID as a length-prefixed UTF-8 string; invalid
  UTF-8 or a truncated request decode to a real `ProtocolError`, not a
  panic.
- **Changed:** `server::AppState` now holds both `log: Arc<Mutex<Log>>` and
  `consumer_offsets: Arc<Mutex<ConsumerOffsets>>`, replacing `serve`'s
  previous bare `Arc<Mutex<Log>>` parameter. Two separate locks, not one
  covering both — a `Commit`/`LastCommitted` request never waits on `Log`'s
  lock, and vice versa. `dispatch` gained the two new match arms; the
  `ConsumerOffsets`'s wire exposure `README.md`/`ARCHITECTURE.md` previously
  flagged as still open now exists.
- 12 new tests (53 total): 8 in `protocol.rs` (round trips for both new
  request/response pairs, an empty consumer ID, a truncated `Commit`
  request, an invalid-UTF-8 consumer ID), 4 in `server.rs` against **real
  loopback TCP sockets** — commit then read back via `LastCommitted`, an
  unknown consumer reads back `None`, a later commit overwrites an earlier
  one for the same consumer, and two consumers committing concurrently stay
  independent (the same cross-connection safety property the `Log` tests
  already covered, now for the other lock).
- **Known limitations, stated plainly:** still no graceful shutdown, still
  no frame-size sanity cap — both called out in the previous entry, neither
  touched by this change.

## Socket integration: `rusty_stream` is now a real TCP server
**2026-08-02**

- **Added:** `src/server.rs`'s `serve` — a real `rusty_tokio::io::
  TcpListener` accept loop, one task per connection, each decoding a framed
  `Request`, dispatching it against a `Log` shared across every connection
  via `rusty_tokio::sync::Mutex`, and writing the framed `Response` back.
  This is the piece `protocol.rs` explicitly deferred — `rusty_stream`
  actually speaks its own wire protocol over a real socket now.
- 3 new tests (41 total), all against **real loopback TCP sockets**, not
  simulated ones (`rusty_tokio` has no network fault injection, only
  `SimDriver` has disk fault injection) — produce-then-fetch round trip,
  fetching an unknown offset returns a real `Error` response instead of
  dropping the connection, and two concurrent clients sharing one `Log`
  land at distinct offsets safely.
- **Known limitations, stated plainly:** no consumer-offset commit request
  in the wire protocol yet (`ConsumerOffsets` has no wire exposure at all);
  no graceful shutdown (`serve` runs until aborted or the listener errors,
  no draining of in-flight connections); no frame-size sanity cap (a client
  claiming a multi-gigabyte body gets that much allocated before the rest
  fails to arrive) — fine for a trusted client, a real deployment needs a
  cap before this is internet-facing.

## Wire protocol integration started: message encoding on `rusty_wire`
**2026-08-02**

- **Added:** `rusty_wire` as a pinned `git` dependency (unpublished, same
  reasoning and pattern as `rusty_tokio` — checked crates.io first, not
  there). `rusty_wire` turned out to be a minimal byte-cursor `Reader`/
  `Writer` utility, not a pre-made protocol — ADR-0002 D1's "extend
  `rusty_wire`" means build `rusty_stream`'s own protocol on its primitives,
  which is what this does.
- **Added:** `src/protocol.rs` — `Request` (`Produce`/`Fetch`) and
  `Response` (`Produced`/`Fetched`/`Error`) encode/decode, matching Phase
  1's actual storage surface (`Log`/`Segment`). Pure and synchronous, same
  "testable without a runtime" shape as `record.rs`. `frame`/`frame_len` for
  the length-prefix a real socket layer will need.
- 12 new tests (38 total): every message type round-trips, plus the same
  "malformed input reports a typed error, never panics" rigor as `record.rs`
  — truncated messages, an unknown opcode/status byte, invalid UTF-8 in an
  error message.
- **Known limitation, stated plainly — this is a start, not the
  integration:** nothing here is wired to a socket. No `rusty_tokio`
  listener, no connection loop decoding a `Request` off the wire,
  dispatching it to a real `Log`, and encoding the `Response` back. That's
  the next real step — `protocol.rs` existing doesn't imply it's done, and
  the module's own docs say so directly rather than leaving it implied.

## Consumer offset tracking: `ConsumerOffsets`, built on `Segment` directly
**2026-08-02**

- **Added:** `src/consumer.rs`'s `ConsumerOffsets` — tracks each consumer's
  last-committed `Offset`, backed by a dedicated `Segment` of commit records
  (`[consumer_id][offset]`) rather than a new storage primitive. `open_on`
  replays every commit and keeps only the last one per consumer
  (last-write-wins), reusing `Segment`'s existing torn-write/checksum
  recovery instead of a second hand-rolled recovery path.
- 5 new tests (26 total): independent tracking across consumers, in-memory
  view updates immediately on commit, recovery correctly replays to the
  latest commit per consumer, and a torn (never-synced) commit is dropped on
  recovery rather than served — the same fault-injection rigor as every
  other module here.
- **Known limitation:** one `ConsumerOffsets` file grows forever — no
  compaction (`docs/phase1-scope.md` §2 already scopes compaction out of
  Phase 1 entirely, so this isn't a surprise, but it's worth naming: a
  long-lived deployment with many consumers committing frequently will want
  this addressed before it matters in practice, not just implicitly deferred
  the way `retention.rs`'s own known limitations are already tracked).

## Retention module: segment rolling, size/time-based deletion, `Clock` seam
**2026-08-02**

- **Added:** `src/retention.rs`'s `Log` — owns a sequence of `Segment`s in
  one directory, rolls to a new one once the active segment would cross
  `RetentionPolicy::max_segment_bytes`, and deletes closed segments via
  `Log::enforce_retention` by size (`max_total_bytes`) or age
  (`max_segment_age_millis`) — oldest first, active segment never touched.
  A retiring segment is synced as part of the roll itself, not left to the
  caller, so a crash right after a roll can't lose records this process
  already considered safely closed.
- **Added:** `src/clock.rs`'s `Clock` trait, `SystemClock` (real),
  `SimClock` (deterministic, manually advanced) — the same
  real/simulated pairing `rusty_tokio::io::OpDriver`/`SimDriver` already
  established, applied to time so retention age checks are provable without
  a test actually sleeping. Unlike `OpDriver`, this is `rusty_stream`'s own
  trait — `rusty_tokio` has no clock abstraction to build on.
- **Added:** `Segment::byte_len()` — what `Log` uses to decide when to roll.
- 7 new tests (21 total), including: crossing the size threshold actually
  rolls; size-based retention deletes the right segment and only that one;
  time-based retention leaves a segment alone until the simulated clock
  actually crosses the age window, then deletes it; and a full
  create-append-roll-crash-reopen cycle recovers both the closed and active
  segment correctly.
- **Known limitation, stated plainly:** `Log::open` recovers from an
  explicit list of segment base offsets, not directory scanning —
  `rusty_tokio::io::OpDriver` has no directory-listing operation at all
  (`SimDriver` only knows paths it's been told about), so this is a real
  constraint, not a shortcut. In practice this means a manifest of which
  segments exist needs to be persisted somewhere before a real
  restart-and-recover path works — not built in this pass. Recovered closed
  segments also don't have a real creation timestamp yet (not persisted
  anywhere on disk), so time-based retention is only accurate within a
  single process's uptime, not across a restart, until segment creation
  time gets added to the on-disk header alongside epoch/base_offset.

## Cargo project scaffolded: first real `Segment` storage code lands
**2026-08-02**

- **Added:** `Cargo.toml` depending on `rusty_tokio` (pinned `git` `rev`,
  `thread-per-core` + `io-uring-fs` features, per ADR-0002 D3) — the first
  real code in this repo.
- **Added:** `src/record.rs` — on-disk record framing (`[len][crc32][payload]`),
  hand-rolled CRC-32/ISO-HDLC (no new dependency — this project treats every
  dependency as audit surface, per ADR-0002's whole D3 thread).
- **Added:** `src/offset.rs` — `Offset`/`DurableOffset`/`CommittedOffset`/
  `Epoch`, the ADR-0002 D2 primitives a future consensus layer needs without
  a storage-format migration.
- **Added:** `src/segment.rs` — a real, working append-only `Segment`: create,
  append, read, sync, and crash recovery (truncates a torn tail rather than
  serving it). Built directly on `rusty_tokio`'s `OpDriver`/`UringFile`, not a
  hand-rolled parallel trait, per ADR-0002 D4. 14 tests pass, including
  working versions of all three of D4's minimal DST scenarios (crash/recovery
  cycles, torn write, lying fsync) against `SimDriver`.
- **Added:** `src/retention.rs`, `src/consumer.rs` — module stubs (docs only,
  no implementation) for segment rolling/deletion and per-consumer offset
  tracking, scoped but deliberately not designed yet.
- **Fixed (blocking, upstream):** `rusty_tokio` could not be consumed as a
  Cargo `git` dependency by any external project — its `Cargo.toml` used
  `path = "../rusty_std"`-style sibling-repo dependencies, which only
  resolve inside its own multi-repo dev checkout. Filed and verified fixed
  upstream (`baileyrd/rusty_tokio#254`, closed via `rusty_std`/`rusty_libc`/
  `rusty_win32` converting to pinned `git` dependencies, matching the
  existing `rustils` pattern) before this scaffold could build at all.
  Re-verified the actual fix — not just that the issue closed — with a
  build against a completely fresh `CARGO_HOME`.
- **Known limitation:** the on-disk index is dense and in-memory only, not
  the sparse on-disk index `docs/phase1-scope.md` §2 describes — real,
  useful, but a narrower slice than the full Phase 1 scope. No fsync policy
  configuration yet (`Segment::sync` exists; when to call it is left to the
  caller). No wire protocol integration (ADR-0002 D1) yet.

## ADR-0002 D3 reopened and reversed: `rusty_tokio` replaces compio as the Phase 1 runtime
**2026-08-01**

- **Changed:** the Phase 1 runtime decision flips from compio (pinned 0.18.0)
  to `rusty_tokio` (RustyMill's own runtime), with its `thread-per-core` and
  `io-uring-fs` features. `rusty_tokio` was previously out of the D3
  comparison entirely — it lacked thread-per-core scheduling and had no
  io_uring file I/O (`fs::File` was 100% `spawn_blocking`). Those two gaps
  were written up as a concrete handoff (`baileyrd/rusty_tokio#252`) and
  filed as an issue; `baileyrd/rusty_tokio#253` closed it with a real
  implementation.
- **Verified, not taken on trust:** builds clean on stable Rust; straced the
  new example and confirmed real `io_uring_enter` calls for file ops (not
  `spawn_blocking`); confirmed genuine per-core reactor instantiation via
  `sched_setaffinity` and source inspection; ran the documented ASAN
  cancellation-safety command for real (clean, no UAF/double-free); ran all
  21 new tests (cancellation, segment-roll, thread-per-core, `SimDriver`
  fault injection, segment-log crash recovery) — all pass; re-measured
  dependency footprint at 28 crates (still roughly a tenth of compio's 231).
- **Bonus:** `rusty_tokio`'s `io-uring-fs` ships an `OpDriver` trait with a
  real `SimDriver` (torn writes, lying fsyncs, disk-full, crash/reopen) —
  exactly what ADR-0002 D4 asked for as a plan, delivered as a working
  implementation. D4 now builds directly on `OpDriver`/`SimDriver` instead of
  hand-rolling a parallel `Storage`/`Clock` trait.
- **Known limitation, stated plainly, not a shortfall:** `rusty_tokio`'s
  `io-uring-fs` uses one process-wide io_uring ring, not one per core —
  a deliberate, documented choice (correctness over per-core I/O throughput),
  consistent with this project's own stance on not chasing throughput
  records. Only the *scheduling* side (tasks, sockets, timers) is fully
  per-core; disk I/O submission still synchronizes through one driver thread.

## ADR-0002 D3: runtime decision closed — compio 0.18.0, after checking all three
**2026-08-01**

- **Changed:** ran the same source-level spike (stable-Rust build + real
  program + driver/reactor source inspection) against glommio 0.9.0 and monoio
  0.2.4 that had just been run against compio, rather than pin-and-move-on
  after finding compio's stated rationale was wrong.
- **Found:** none of the three runtimes expose a public seam for injecting
  simulated per-operation disk I/O — glommio's `Reactor` is even more
  monolithic than compio's (`pub(crate)`, single non-generic `Rc`, no
  alternate-backend cfg selection at all); monoio's `Driver` trait is
  genuinely public and `Runtime<D>` is generic over it, but that trait only
  covers executor scheduling, not the actual read/write/fsync dispatch (which
  is `pub(crate)` and hardcoded to a closed enum). This neutralizes
  DST-pluggability as a differentiator entirely — D4's own `Storage`/`Clock`
  trait abstraction is what actually delivers DST, regardless of runtime.
- **Also reconfirmed, not stale:** DataDog/glommio#707 ("call for
  maintainers") is still open and unresolved; monoio's io_uring feature-parity
  gap and slow maintenance pace (flagged by Iggy) still hold as of a fresh
  2026-08-01 check.
- **Decided:** compio, pinned to exactly `0.18.0` (not a range), stays the
  Phase 1 runtime — now on the strength of dependency modularity and
  maintainer responsiveness rather than the falsified driver-swappability
  claim. D3 in ADR-0002 is closed, not provisional.

## ADR-0002 D3: compio validation spike — core rationale falsified, decision reopened
**2026-08-01**

- **Changed:** ran the D3 validation spike the ADR called for. Built and ran a
  minimal compio (0.19.1) file-I/O program in this environment and read the
  actual `compio-driver`/`compio-runtime` source rather than relying on docs.
- **Fixed (a wrong claim in the original ADR):** D3's stated reason for
  preferring compio over glommio/monoio — "the only one of the three engineered
  to let the I/O driver be swapped for a simulated one" — is false. `Proactor`
  and `Runtime` are non-generic, and the `Driver` type is chosen entirely at
  compile time from a closed set of built-in backends; there's no public trait
  a downstream crate can implement to inject a simulated driver. This does not
  block D4's testing strategy (which abstracts at the team's own `Storage`/
  `Clock` trait boundary, not the runtime's internals), but the specific reason
  given for picking compio was wrong and needed correcting, not quietly kept.
- **Found (new, unplanned):** current compio (0.19.0/0.19.1) does not compile
  on stable Rust — an unconditional use of the newly-and-not-yet-stably
  stabilized `cfg_select!` std macro. Confirmed via bisection: `compio` 0.18.0
  builds cleanly on stable, 0.19.0 does not. Likely temporary (stable Rust's
  ~6-week release cycle should catch up), but real today.
- **Known limitation:** D3 is intentionally left open rather than force-closed
  — the ADR now names two concrete paths to finalize it (pin to 0.18.0 and
  accept the non-pluggable-driver finding, or re-run this same source-level
  check against glommio/monoio) rather than picking one without the same rigor
  applied to the alternatives.

## ADR-0002: Phase 1 foundational decisions
**2026-08-01**

- **Added:** `docs/adr/0002-phase1-foundational-decisions.md` — resolves all four
  `docs/phase1-scope.md` §6 open questions with cited research: skip Kafka
  wire-protocol compatibility (build on `rusty_wire`); defer the VSR-vs-Raft
  choice to Phase 2 but lean VSR and require consensus-ready storage primitives
  now (durable/committed-offset split, truncatable log tail, epoch/fencing-token
  field); compio as a provisional runtime choice pending a validation spike;
  coexist with NATS JetStream behind an explicit, criteria-based re-evaluation
  gate rather than replacing it outright.
- **Added:** a concrete DST testing strategy (injectable `Storage`/`Clock`
  traits from the first storage-engine commit, three minimal fault-injection
  tests) and a set of "consumer gates" the storage engine must clear before
  Phase 1 is considered done.
- **Known limitation:** the runtime choice (D3) is explicitly provisional —
  the ADR names a validation spike that hasn't run yet. No implementation
  lands in this change; this is research/ADR only, per the scope doc's gate.

## Repo setup — minimal CI workflow + main branch
**2026-08-01**

- **Added:** `main` branch on `origin`, created from the governance-scaffolding
  commit and now the repo's default branch.
- **Added:** `.github/workflows/ci.yml` — a minimal `check` job (name matches the
  required-status-check convention in the repo-config reference) that no-ops green
  until a `Cargo.toml` exists, then automatically runs `cargo fmt`/`clippy`/`test`.
  Exists now so branch protection has a real check to gate on rather than one
  that's never reported.
- **Known limitation:** branch protection on `main` is still unset — no tool in
  this environment reaches GitHub's branch-protection or repo-settings API, so
  that (require PR, require the `check` status, require up-to-date branches, and
  disabling squash/rebase merge in repo settings) remains a manual step.

## Repo setup — Phase 1 scope doc + governance scaffolding
**2026-08-01**

- **Added:** trimmed copy of the Phase 1 pre-RFC research brief
  (`docs/phase1-scope.md`) — dropped the "governance-native data contracts"
  differentiator per direction to leave that out for now.
- **Added:** standard governance file set via `repo-config` — README, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue templates, ADR seed.
- **Known limitation:** no `Cargo.toml` yet, so no CI workflow was added — nothing
  to run. `ARCHITECTURE.md`'s boundary table is left as scaffold since no code has
  landed. This repo has no `main` branch yet (root commit lives on
  `claude/review-attached-document-3at8q9`), so the PR-per-change workflow doesn't
  apply until one exists.
