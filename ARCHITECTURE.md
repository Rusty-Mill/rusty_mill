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
| `protocol::{encode_request, decode_request, encode_response, decode_response}` | pure functions, no I/O, built on `rusty_wire::{Reader, Writer}` | The wire-protocol boundary (ADR-0002 D1) — same "pure and synchronous, testable without a runtime" shape as `record`. |
| `server::serve` | `rusty_tokio::io::{TcpListener, TcpStream}` | The socket adapter: accepts connections, decodes framed `Request`s, dispatches against a shared `Log`, encodes `Response`s back. See "Data flow" below for what's still missing (consumer-offset commits have no wire exposure, no graceful shutdown). |

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
via `Clock`) — oldest first, active segment never touched. `Log::open` recovers
from an explicit list of segment base offsets rather than scanning the
directory, because `OpDriver` has no directory-listing operation at all (see
`retention.rs`'s own docs for the manifest-persistence gap this leaves open).

`consumer::ConsumerOffsets` tracks each consumer's last-committed offset the
same way: `commit` encodes `[consumer_id][offset]` as one record and appends
it to a dedicated `Segment`, rather than a second storage engine. `open_on`
replays every commit record and keeps only the last one per consumer
(last-write-wins) — reusing `Segment`'s existing torn-write/checksum recovery
rather than a second hand-rolled recovery path.

`protocol::{Request, Response}` encode/decode to `[opcode/status][body]` using
`rusty_wire::{Reader, Writer}` for every primitive read/write — `Produce`
(append a payload) and `Fetch` (read one record back), matching Phase 1's
actual storage surface. `frame`/`frame_len` add the 4-byte length prefix a
real socket layer needs to know how many bytes make up one message.

`server::serve` is that socket layer: accepts connections on a
`rusty_tokio::io::TcpListener`, spawns one task per connection, and each
connection loops reading a 4-byte length header, reading that many more
bytes, decoding a `Request`, dispatching it against one `Log` shared across
every connection via `rusty_tokio::sync::Mutex` (a real cross-core-safe lock,
not an optimization to defer — nothing about the thread-per-core runtime
guarantees two connections land on the same core), and writing the framed
`Response` back. A connection ends cleanly on `UnexpectedEof` (the peer
disconnected) or propagates a real I/O error. Verified against real loopback
TCP sockets and a real `Log` (backed by `SimDriver`) in `server.rs`'s own
tests — `rusty_tokio` has no network fault injection, only `SimDriver` has
disk fault injection, so this is real networking, not simulated.

**Still missing, stated plainly:** no consumer-offset commit request in the
wire protocol — `ConsumerOffsets` has no wire exposure at all yet. No
graceful shutdown. No frame-size sanity cap (a client claiming a
multi-gigabyte body gets that much allocated before the read fails) — fine
for a trusted client, not yet for anything internet-facing.

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
