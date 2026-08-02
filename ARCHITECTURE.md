# Architecture

## Overview
`rusty_stream` is a single-node, append-only durable log: segment files plus a
sparse offset index (Kafka's `.log`/`.index` model), extending `rusty_wire` for the
wire protocol rather than starting a new one. Phase 1 is deliberately scoped to one
node with no replication — see [docs/phase1-scope.md](./docs/phase1-scope.md) for
the full research brief and the differentiators this is meant to pursue instead of
just re-deriving Kafka (sovereignty-first deployment, DST-first testing, a
deliberate rather than reflexive consensus choice for Phase 2).

## Boundaries
<!-- Domain logic vs. I/O and framework details (ports-and-adapters).
     List the ports (interfaces) and the adapters that implement them. -->

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `rusty_tokio::io::OpDriver` | `IoUringDriver` (real, production), `SimDriver` (deterministic, seeded fault injection) | Not our own trait — `segment::Segment` builds directly on `rusty_tokio`'s seam (ADR-0002 D3/D4) rather than a parallel hand-rolled one. Swapped at the call site (`Segment::create_on`/`open_on` take `Arc<dyn OpDriver>`), not via config. |
| `record::{encode, decode}` | pure functions, no I/O | The framing/checksum boundary — deliberately synchronous and driver-independent so it's unit-testable without any runtime at all (see `record.rs`'s own tests). |
| `offset::{DurableOffset, CommittedOffset, Epoch}` | — (value types, no adapter) | The ADR-0002 D2 primitives a future consensus layer attaches to; `segment::Segment` is the only thing that currently produces/consumes them. |
| `clock::Clock` | `SystemClock` (real, production), `SimClock` (deterministic, manually advanced) | Exists for the same reason `OpDriver` does — `retention::Log`'s time-based retention has to be provable without a test actually sleeping. Our own trait (unlike `OpDriver`): `rusty_tokio` has no clock abstraction to build on here. |
| `consumer::ConsumerOffsets` | a dedicated `Segment` of commit records | Not a new storage primitive — reuses `Segment`'s own append/recover machinery for consumer-offset commits, replayed last-write-wins on open. See "Data flow" below. |
| `manifest::Manifest` | a dedicated `Segment` of `Opened`/`Deleted` events | What `retention::Log::open` reads to discover which segments exist — `rusty_tokio::io::OpDriver` has no directory-listing operation, so this is that discovery made durable. Same reused-`Segment` pattern as `ConsumerOffsets`. See "Data flow" below. |
| `protocol::{encode_request, decode_request, encode_response, decode_response}` | pure functions, no I/O, built on `rusty_wire::{Reader, Writer}` | The wire-protocol boundary (ADR-0002 D1) — same "pure and synchronous, testable without a runtime" shape as `record`. Four request types: `Produce`/`Fetch` against the log, `Commit`/`LastCommitted` against a consumer's offset. |
| `server::serve` | `rusty_tokio::io::{TcpListener, TcpStream}` | The socket adapter: accepts connections, decodes framed `Request`s, dispatches against `AppState`'s `Log` and `ConsumerOffsets` (each behind its own lock), encodes `Response`s back. See "Data flow" below for what's still missing (no graceful shutdown, no frame-size cap). |
| `client::Client` | `rusty_tokio::io::TcpStream` | The client-side counterpart of `server::serve` — `docs/phase1-scope.md` §2's "Client SDK — Rust first," the last item on Phase 1's scope list. One `Client` per connection, `produce`/`fetch`/`commit`/`last_committed` methods matching the four `Request` variants one-to-one. |

## Structure
<!-- Greenfield default (see references/scan-and-defaults.md): modular monolith,
     composition over inheritance, ports-and-adapters keeping domain logic free of
     I/O and framework details. A component gets split into its own service only for
     a concrete forcing function — independent scaling, a team/language boundary, or
     hard fault isolation. Note here if/why this repo has already crossed that line. -->

## Data flow
`Segment::append` encodes a payload (`record::encode` — length + CRC32 framing),
writes it at the segment's current write position via `UringFile::write_at`, and
returns the new record's `Offset`. Nothing is synced to disk until `Segment::sync`
calls `fsync` explicitly and returns the new `DurableOffset` — callers choose when
to sync, not `append` itself (configurable fsync policy, `docs/phase1-scope.md`
§2, is a Phase 1 follow-up: the seam exists, a policy on top of it doesn't yet).

`Segment::open_on` is the recovery path: replays every record from the last known
header to EOF, and truncates the file (`set_len`) at the first record that fails
to decode — a torn write or a checksum mismatch — rather than serving a partial
or corrupt record. This is exercised directly by `segment.rs`'s own tests against
`SimDriver`'s fault injection (torn writes, lying `fsync`, crash-and-reopen),
matching ADR-0002 D4's three minimal DST scenarios.

`retention::Log` owns a sequence of `Segment`s and drives the same flow across
more than one: `Log::append` rolls to a new segment once the active one would
cross `RetentionPolicy::max_segment_bytes`, syncing the retired segment before
it's ever read from again (so a crash right after a roll can't lose records this
process already considered safely closed). `Log::enforce_retention` then deletes
closed segments by size (`max_total_bytes`) or age (`max_segment_age_millis`,
via `Clock`) — oldest first, active segment never touched.

`manifest::Manifest` is what makes `Log::open` possible without scanning the
directory (`OpDriver` has no directory-listing operation at all): a dedicated
`Segment` of `Opened`/`Deleted` events, replayed on open to reconstruct which
segments currently exist and when each was created — reusing `Segment`'s own
recovery machinery rather than a second hand-rolled one, the same way
`ConsumerOffsets` does. Every manifest event is synced immediately (unlike a
regular record append), and a segment's manifest event is always ordered
around its file on disk so a crash between the two leaves only a harmless
orphan, never a phantom reference: `Log` creates a new segment file *then*
calls `record_opened`, and calls `record_deleted` *before* actually removing
a segment file. Recovering each segment's real creation time from the
manifest also means time-based retention stays accurate across a restart,
not just within one process's uptime.

`consumer::ConsumerOffsets` tracks each consumer's last-committed offset the
same way: `commit` encodes `[consumer_id][offset]` as one record and appends
it to a dedicated `Segment`, rather than a second storage engine. `open_on`
replays every commit record and keeps only the last one per consumer
(last-write-wins) — reusing `Segment`'s existing torn-write/checksum recovery
rather than a second hand-rolled recovery path.

`protocol::{Request, Response}` encode/decode to `[opcode/status][body]` using
`rusty_wire::{Reader, Writer}` for every primitive read/write — `Produce`
(append a payload) and `Fetch` (read one record back) against the log,
`Commit` (persist a consumer's offset) and `LastCommitted` (read it back,
`Option<Offset>` — `None` for a consumer that's never committed, not an
error) against `ConsumerOffsets`. `frame`/`frame_len` add the 4-byte length
prefix a real socket layer needs to know how many bytes make up one message.

`server::serve` is that socket layer: accepts connections on a
`rusty_tokio::io::TcpListener`, spawns one task per connection, and each
connection loops reading a 4-byte length header, reading that many more
bytes, decoding a `Request`, dispatching it against `AppState` — a `Log` and
a `ConsumerOffsets`, each shared across every connection via its own
`rusty_tokio::sync::Mutex` (real cross-core-safe locks, not an optimization
to defer — nothing about the thread-per-core runtime guarantees two
connections land on the same core; two separate locks so a `Commit` never
waits on `Log`'s lock and vice versa) — and writing the framed `Response`
back. A connection ends cleanly on `UnexpectedEof` (the peer disconnected) or
propagates a real I/O error. Verified against real loopback TCP sockets and a
real `Log`/`ConsumerOffsets` (both backed by `SimDriver`) in `server.rs`'s
own tests — `rusty_tokio` has no network fault injection, only `SimDriver`
has disk fault injection, so this is real networking, not simulated.

**Still missing, stated plainly:** no graceful shutdown. No frame-size
sanity cap (a client claiming a multi-gigabyte body gets that much allocated
before the read fails) — fine for a trusted client, not yet for anything
internet-facing.

`client::Client` is `server::serve`'s counterpart: `Client::connect` opens
one `TcpStream`, and `produce`/`fetch`/`commit`/`last_committed` each encode
a `Request`, frame it, write it, read one framed `Response` back, and decode
it — the same `frame`/`encode_request`/`decode_response` primitives
`server.rs`'s own tests used directly before this existed. A
`Response::Error` from the server is unwrapped into `ClientError::Server`
rather than left for every caller to match on individually. One `Client` is
not safe to share across concurrent callers — nothing multiplexes responses
back to a specific in-flight request on one connection — so concurrent use
means one `Client` per task, the same tradeoff `AppState` makes explicit on
the server side.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
Phase 1 explicitly excludes (see docs/phase1-scope.md §2):
- Multi-broker replication / clustering
- Kafka wire-protocol compatibility layer
- WASM-based stream transforms
- Consumer group rebalancing

These are Phase 2+ candidates, gated on an actual forcing function rather than
built speculatively.
