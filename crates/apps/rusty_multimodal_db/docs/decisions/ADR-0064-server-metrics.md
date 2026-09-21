# ADR-0064: Server Metrics — `Request::Metrics`, Prometheus Text Over the Existing Wire

- Status: **Accepted as designed and implemented** (2026-09-14 — the
  owner picked option (a): a bounded, fixed counter set behind one
  gated request, Prometheus text, no new dependency). Proposed and
  implemented in the same session.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/design/SERVER-METRICS-DESIGN.md` (the full design),
  `docs/FUTURE-GROWTH.md`'s "Operational maturity" section (this
  round's source — a gap named for the first time this session, not
  previously declined), `ADR-0043` (client ecosystem — the wire-first,
  no-new-dependency posture this round follows), `ADR-0032`
  (`ServeOptions` — the cross-cutting-concern precedent this round's
  counters handle mirrors the shape of, deliberately kept separate
  from), `ADR-0031`/`ADR-0029` (access log / audit log — this crate's
  two existing observability sinks, neither of which answers "is this
  process healthy right now" without reading a file).
- Supersedes/Superseded by: none. Additive: `Request::Metrics` (new
  variant), `Response::Metrics` (new variant), `PROTOCOL_VERSION` bump,
  `src/server/metrics.rs` (new). No existing request, file format, or
  client behavior changes.

## Context

`docs/FUTURE-GROWTH.md`'s new "Operational maturity" section (added
this session, `ADR-0062`'s Definition of Done having already closed
this crate's consumer-facing correctness bar) names a real gap: no
counters/gauges surface exists anywhere in this crate. The audit log
(`ADR-0029`) and access log (`ADR-0031`) both write structured lines to
a file, on every admission/auth decision or every request respectively
— real, but neither answers a live "how many requests has this process
served, how many failed, how many connections are open right now"
question without parsing that file after the fact. (The Prometheus-text
metrics the differential test suite exercises, `ADR-0062` item 5,
belong to the *consumer's* hub layer, `rusty_remind_me` — a separate
process, a separate codebase. This crate itself has never exposed
anything comparable.)

Two things make this bounded rather than a from-scratch build:

1. **One call site already sees every request.** `handle_connection`
   records exactly one `AccessEvent` per dispatched request today
   (`ADR-0031`) — the identical location a metrics increment belongs
   at, no new hook needed.
2. **`ServeOptions` already establishes the shape for a cross-cutting,
   opt-in `serve` concern** (`ADR-0032` consolidated five of them into
   one config type). A live counters handle is a different *kind* of
   thing — mutable runtime state, not configuration — so it is proposed
   as a sibling `Arc<ServerMetrics>` passed alongside `ServeOptions`
   rather than folded into it, the same distinction `journal:
   Option<CommitGroup>` already draws between per-adapter config and
   per-adapter live state.

## Decision

Propose a new, field-less `Request::Metrics` answered by
`Response::Metrics { text: String }` — Prometheus text exposition
format, hand-formatted (no new dependency; the format is a well-known,
simple text grammar) from a small, fixed set of process-wide atomic
counters: total requests, ok/error split, total connections accepted,
currently-active connections. Gated as a **read** — authentication
required when configured, but **not** restricted to a `ReadWrite`
token, the first request this crate gates more permissively than the
existing read/write split. Rendered fresh on every call from live
atomics; nothing persisted, nothing surviving a restart.

The fork, held for the owner:

- **(a) As scoped above — a bounded, fixed counter set behind one
  gated request, Prometheus text, no new dependency (recommended).**
  Smallest real answer: one new request/response pair, one new small
  module, one new increment at an already-existing call site. Fixed
  cardinality (six counters, never growing with new `Request`
  variants) is a deliberate constraint, not an oversight — see the
  design document's "Considered options" (b) for the per-`RequestKind`
  alternative this declines for now.
- **(b) A separate HTTP `/metrics` listener**, directly scrapeable by a
  stock Prometheus install with no adapter. Real operational
  convenience, at the cost of a second bound port, a new HTTP-server
  dependency, and a second transport needing its own security story
  (its own TLS? its own auth? shared with `ServeOptions` or not?) —
  exactly the kind of new surface `ADR-0043` found this crate never
  needed for the client ecosystem question either (the wire protocol
  is already trivially bridgeable from any language, including to
  HTTP, entirely outside this crate).
- **(c) Decline.** The gap stays named in `FUTURE-GROWTH.md`, not
  closed. The audit/access logs remain the only observability surface.
  A legitimate answer if the owner judges live metrics not worth a
  protocol-version bump yet, distinct from a genuine architectural
  objection.

## Consequences

- Positive (a): closes a real, now-named gap with no new dependency, no
  new transport, and a fixed, small increase in per-request cost (a
  handful of atomic increments already on the hot path where
  `AccessEvent` recording already happens). Every server answers it
  identically — no per-domain `Unsupported` branch, unlike every other
  operator request this crate has added (`Compact`, and `ADR-0065`'s
  proposed `Backup`).
- Named, not hidden: this is **not** a full observability story. No
  latency percentiles (*since `ADR-0088`: a fixed-bucket histogram*),
  no per-request-kind breakdown, no
  Prometheus-native scrape endpoint an out-of-the-box Prometheus
  install can hit without a bridge script. It answers exactly "is this
  process up, how loaded, how much is failing" — a real, useful, but
  bounded slice.
- Named, not hidden: gating a request by "authenticated, any class"
  rather than the existing `ReadOnly`/`ReadWrite` split is a new
  gating shape. If a future request wants the same treatment, this ADR
  is the precedent to point at; if none ever does, it stays a
  one-off, which is fine.
- This is a **wire, append-only, hard-to-reverse-once-shipped** change
  (`PROTOCOL_VERSION` bump, new variants) — exactly the class of
  decision `WORKFLOW.md` requires design-first, owner-accepted before
  implementation, which is why this is a proposal, not code.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/metrics.rs` (new — `ServerMetrics`, six atomics,
  `render()`, `ConnectionMetricsGuard`), `src/server/protocol.rs`
  (`Request::Metrics`/`Response::Metrics`, `PROTOCOL_VERSION` 22 → 24,
  shared with `ADR-0065` implemented in the same round — this ADR
  claims protocol 23), `src/server/serve.rs` (`handle_connection`'s
  connection-open/close tracking and the shared post-dispatch
  `record_request` call, gated like `Page`/`CountEdges` — a read, no
  `ReadOnly` restriction), `src/server/client.rs`
  (`SchemaDrivenClient::metrics()`). `SERVER-001` v0.53.0 / FR-063.
- **One real correction, found during implementation, before writing
  the config-threading code the Decision above sketched**: the
  Decision proposed a separate `Arc<ServerMetrics>` parameter passed
  alongside `ServeOptions`, reasoning that `ServeOptions` was
  configuration, not live state. Implementing it found that reasoning
  already false against this crate's own code: `ServeOptions::rate_limit`
  is `Option<Arc<FailureTable>>` — a `Mutex<HashMap<..>>`, genuinely
  live, mutated-per-request shared state — sitting inside
  `ServeOptions` since `ADR-0030`, not beside it. `ServerMetrics` is
  the identical shape of thing. Implemented as a plain field,
  `metrics: ServerMetrics`, inside `ServeOptions` (accessed via
  `ServeOptions::metrics()`), with **no `serve`/`serve_tables` signature
  change at all** — simpler than the Decision's own sketch, and it
  keeps faith with `ADR-0032`'s own consolidation (four `serve`
  parameters down to one `ServeOptions`) rather than reopening it for
  a second live-state parameter.
- Proven: `metrics.rs`'s four unit tests (the `requests_total ==
  ok + err` invariant, the connection-guard drop-decrements property,
  deterministic rendering); two new `protocol.rs` golden vectors
  (`Request::Metrics` at index 32/protocol 23, `Response::Metrics` at
  index 21); `tests/server_metrics_integration.rs` (3 tests, real
  socket) — request/ok/err counts measured as a delta across a real
  `Insert`-`Duplicate` refusal (not a client-side-refused write, which
  never reaches the server at all), a `ReadOnly` token answering
  `Metrics` successfully while a real write is `Unauthorized`, and
  `connections_active` returning to `0` after a disconnect. `cargo fmt
  -p rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --all-features -- -D warnings` clean; `cargo test
  -p rusty_multimodal_db --all-features --no-fail-fast` 527 lib tests +
  every integration target green (including `server_python_client`,
  fixed in a follow-up round the same session — see `ADR-0043`'s own
  final addendum).
