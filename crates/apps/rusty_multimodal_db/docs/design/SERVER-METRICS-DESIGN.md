# Server Metrics: `Request::Metrics`, a Bounded Counter Set, Prometheus Text on the Existing Wire (Accepted)

- Status: **Accepted as designed, implemented as scoped** (2026-09-14,
  `ADR-0064`, option (a)). See `ADR-0064`'s own "Acceptance and
  implementation" for the one real deviation found while building it
  (`ServerMetrics` lives inside `ServeOptions` as a plain field, not a
  sibling `Arc` parameter this document's own "Proposed shape" section
  sketched — `serve`/`serve_tables`'s signatures are unchanged).
- Date: 2026-09-14
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section
  (this round's own source: "Metrics/observability at the
  storage-engine layer... there is no counters/gauges surface... and no
  `/metrics`-style endpoint"), `ADR-0043` (client ecosystem — the
  stdlib-only, wire-first posture this round follows), `ADR-0029`/
  `ADR-0031` (audit log / access log — the two existing observability
  sinks this is deliberately a third, independent thing from, not a
  reuse of either).
- Supersedes/Superseded by: none. Additive: one new `Request`/`Response`
  variant pair, one new small module (`src/server/metrics.rs`), a
  `ServeOptions`-adjacent counters handle threaded through
  `handle_connection`. No file-format change, no existing request's
  behavior changes.

## Purpose and scope

Every request the server answers today is invisible from the outside
except through the audit log (admission/auth decisions) and the access
log (one line per request, opt-in, to a file). Neither answers "is this
process healthy right now" without parsing a log file. This round adds
exactly that: a small, fixed set of process-wide counters, readable
over the same TCP/TLS connection every other request already uses, as
Prometheus text exposition format — the de facto standard scrape format,
plain enough to hand-format with no new dependency.

Scope, exactly: one new gated read request answering a bounded set of
counters as text (`MET-FR-001`–`003`); the counters themselves,
incremented at the one place every request already passes through
(`MET-FR-002`); the text format and its golden-vector test
(`MET-FR-004`); gating (`MET-FR-005`).

## Non-goals

- **A separate HTTP `/metrics` listener.** Would need a second bound
  port, an HTTP server (a new dependency — this crate has never taken
  one for the server layer beyond `rusty_tls`), and a second transport
  to secure. `ADR-0043`'s own finding — "the protocol is already
  trivially implementable in any language" over the one existing
  TCP/TLS listener — applies here too: a metrics *request* over the
  wire everything else already uses is strictly less new surface than a
  second listener.
- **Per-request-kind cardinality.** A counter per `RequestKind` (23 and
  growing) is tempting but this round declines it — see "Considered
  options." The counters below are a small, fixed set chosen so total
  cardinality never grows as new `Request` variants are added.
- **Latency histograms/percentiles.** Needs bucketing machinery and a
  real decision about bucket boundaries; a genuinely separate, larger
  round if ever wanted. `benches/server.rs` already answers "how fast"
  offline; this round answers "is it up and how loaded," not "how
  fast right now."
- **Per-connection or per-peer metrics.** The access log (`ADR-0031`)
  already answers "what did peer X do" at the audit-trail level; this
  round is process-wide aggregate only.
- **Persisting metrics across restarts.** Every counter resets to zero
  on process start, same as any in-memory Prometheus client library's
  default behavior.

## Context and terminology

Read from `src/server/{mod,protocol,access,audit}.rs` as they stand at
`SERVER-001` v0.52.0:

- `handle_connection` (`src/server/serve.rs`) is the one place every
  request already passes through exactly once — `ADR-0031`'s own
  `AccessEvent` is recorded there, after dispatch, before the response
  is sent. The identical call site is where a metrics increment belongs
  — no new pass over the request, no new hook.
- `ServeOptions` (`ADR-0032`) is already the one place cross-cutting,
  opt-in `serve` concerns live — tokens, certificate classes, audit,
  rate limit, access log, TLS. A metrics counters handle is the same
  shape of thing.
- `Request`/`Response` (`src/server/protocol.rs`) currently top out at
  `Request::WriteBatch` (variant index 31 / `0x1f`) and
  `Response::BatchResults` (variant index 20 / `0x14`), `PROTOCOL_VERSION
  = 22`.
- No existing request answers a value that isn't scoped to one table's
  records — `Metrics` would be the first process-wide (not
  `ConnectionStore`-routed) read. It is answered directly by
  `handle_connection`, never reaching `dispatch`/`ConnectionStore` at
  all — every server, `Dog`/`Order`/`Employee`/`Reminder`/`Entity`/
  `Memory`/`Relation` alike, answers it identically, so no
  `Unsupported` case exists for it (unlike `Compact`/`CountEdges`,
  which are per-domain-capability-gated).

## Requirements

- `MET-FR-001` **`Request::Metrics` / `Response::Metrics`.** A new,
  field-less `Request::Metrics` at the next protocol version (23,
  provisional — see "Open questions" on sequencing against
  `ADR-0065`'s `Request::Backup`), answered `Response::Metrics { text:
  String }`. Gated as a **read**: authentication required when
  configured (same default-open behavior as every other request),
  **not** `Unauthorized` for a `ReadOnly` token (observing process
  health is not a data write) — the first request kind gated more
  permissively than `Query`/`Page`, named explicitly rather than
  reusing the write gate by default. `Malformed` below the new
  protocol version (rule 3). Never overlaid by a session (process-wide
  data, not table data — matches `Page`/`CountEdges`'s own "never
  read-set-tracked" precedent).
- `MET-FR-002` **A bounded, fixed counter set.** `src/server/metrics.rs`
  (new): `ServerMetrics { requests_total: AtomicU64, requests_ok_total:
  AtomicU64, requests_err_total: AtomicU64, connections_total:
  AtomicU64, connections_active: AtomicI64, started_at: SystemTime }` —
  six fields, no per-label breakdown, cardinality fixed at compile
  time. `connections_active` increments on accept, decrements on the
  same `Drop` guard `ADR-0029`'s `Disconnected` audit event already
  uses (one guaranteed decrement per accepted connection, not one per
  `return`).
- `MET-FR-003` **Incremented at the existing single dispatch point.**
  `handle_connection`'s post-dispatch line (where `AccessEvent` is
  already recorded, `ADR-0031`) increments `requests_total` and either
  `requests_ok_total` or `requests_err_total` from the same `Outcome`
  value already computed there — zero new branching, one shared
  `Arc<ServerMetrics>` reference.
- `MET-FR-004` **Prometheus text exposition format, deterministic.** A
  pure function `ServerMetrics::render(&self) -> String` — one `# HELP`
  plus `# TYPE` pair per metric, then one sample line, metric names
  fixed and prefixed `dogserver_` (matching Prometheus's own naming
  convention: `<subsystem>_<name>_<unit>`, e.g.
  `dogserver_requests_total`, `dogserver_connections_active`), fields
  emitted in a fixed, tested order — a golden-text fixture test, the
  same discipline `protocol.rs`'s golden byte vectors already use for
  wire stability, applied to text stability instead.
- `MET-FR-005` **Gating.** `Malformed` below the negotiated protocol
  version (rule 3); authentication gate only, no class check;
  `SessionOpen` is **not** a refusal here — unlike every write, reading
  metrics mid-session is harmless and answering it does not touch
  `ConnectionStore` at all.

## Considered options

- **(a) As scoped above — a bounded, fixed counter set behind one gated
  request, rendered as Prometheus text (recommended).** Smallest real
  answer to a genuine gap; no new dependency; reuses the one existing
  transport and the one existing per-request hook.
- **(b) Per-`RequestKind` counters** (23 labels today, growing with
  every future `Request` variant). More useful for diagnosing *which*
  request type is loaded, but cardinality grows every time this crate
  adds a request — the one thing a metrics surface should not do by
  default. Could be added later as a genuinely separate, opt-in,
  bounded-by-a-fixed-enum extension once the base counters prove the
  wire mechanism; not this round.
- **(c) A separate HTTP `/metrics` endpoint** (a second listener, a new
  HTTP-server dependency). Directly scrapeable by a stock Prometheus
  install with no adapter script — a real advantage — but a second
  transport to secure (does it get its own `ServeOptions`? Its own
  TLS? Its own auth?), and a new dependency this crate has avoided at
  every prior server-ecosystem decision (`ADR-0043`). An operator can
  already bridge this trivially: a one-file sidecar script that speaks
  this crate's own wire protocol (exactly `ADR-0043`'s own point) and
  re-serves the text over HTTP, with no new dependency inside this
  crate at all.
- **(d) Decline.** Leaves the gap `FUTURE-GROWTH.md` now names exactly
  where it is; the audit/access logs remain the only observability
  surface, both requiring log-file access rather than a live query.

## Proposed shape

`src/server/metrics.rs` (new, `pub` under `server`): `ServerMetrics`,
`ServerMetrics::new()`, six `record_*`/`increment_*` methods, `render`.
`src/server/protocol.rs`: `Request::Metrics`, `Response::Metrics {
text: String }`, `PROTOCOL_VERSION` bump, golden vectors.
`src/server/serve.rs`: `handle_connection` gains one arm; `serve`
constructs one `Arc<ServerMetrics>` per listener, threaded alongside
`ServeOptions` (not folded into it — `ServeOptions` is Clone/config
data, `ServerMetrics` is live mutable state; keeping them distinct
mirrors `journal: Option<CommitGroup>` living on the adapter rather
than folded into config). `SchemaDrivenClient::metrics() -> Result<String,
ClientError>` on both clients.

## Data/state and invariants

- `requests_total == requests_ok_total + requests_err_total` always
  (checked by the crash/soak-style unit test — a straight arithmetic
  invariant on atomics incremented from one call site).
- `connections_active` never goes negative (the drop-guard discipline
  `ADR-0029` already established for exactly this shape of bug).
- No metric depends on request *content* (no record id, no field
  value, no token) — the same privacy posture the audit log already
  holds itself to.

## Errors, failure, recovery, and observability

No new `ErrorCode`. A `Metrics` request itself never fails once past
the version/auth gate — rendering six atomics to text cannot error.

## Security, privacy, and compatibility

Read-only, no record data, no path argument, no filesystem access — the
smallest possible new attack surface (contrast `ADR-0065`'s
`Request::Backup`, which does write files and needs a real path-
confinement design). A pre-this-round client is unaffected; a
version-negotiated-below-23 client never sees `Request::Metrics` exist.

## Acceptance criteria

1. `ServerMetrics`'s six-field invariant (`requests_total ==
   requests_ok_total + requests_err_total`) holds after a mixed
   sequence of successful and refused requests, verified by a unit
   test driving `handle_connection` directly (the `ADR-0031` access-log
   integration test's own harness shape).
2. `render()`'s output is deterministic and matches a golden text
   fixture, byte for byte, across two calls with the same counter
   state.
3. A `ReadOnly`-token connection can call `Metrics` successfully (proof
   the gate is read-only, not write-gated) while `Compact`/`Delete`
   remain `Unauthorized` for the same token, in the same test.
4. `cargo test --all-features` green; `cargo clippy --all-features --
   -D warnings` clean; `cargo fmt --all --check` clean.

## Verification plan

`cargo test --all-features` (new unit tests in `metrics.rs`, a golden
vector pair in `protocol.rs`, one integration test in
`tests/server_metrics_integration.rs` exercising a real socket:
several mixed requests, then `Metrics`, asserting the rendered counts
match); `cargo clippy --all-features -- -D warnings`; `cargo fmt --all
--check`.

## Traceability

- Roadmap: `SERVER-METRICS-DESIGN` (this document), `SERVER-METRICS`
  (implementation, `Implemented`).
- Decision: `ADR-0064`.
- Specification: `SERVER-001` v0.53.0 / FR-063.
- Requirements: `MET-FR-001`–`005`.

## Open questions

- **Sequencing against `ADR-0065` (`Request::Backup`).** Both rounds
  are proposed together; whichever is implemented first claims
  protocol version 23 and the next `Request`/`Response` variant
  indices, the other becomes 24. Not decided here — an implementation-
  order choice, not a design one.
- **Per-`RequestKind` counters** — option (b) above, left as a genuine
  future extension once the base mechanism is proven, not a promise.
- **Whether metrics should be gated at all** when a server has no
  tokens configured (today's answer: same default-open behavior as
  every other request) — revisit if an operator ever wants metrics
  visible to monitoring infrastructure that cannot hold a data token,
  a real but currently hypothetical split.

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch — protocol 23, `SERVER-001` v0.53.0 / FR-063. See `ADR-0064`'s
  own "Acceptance and implementation" for the real mechanism (a plain
  `ServeOptions` field, not a sibling parameter) and why it changed
  from this document's own sketch.
