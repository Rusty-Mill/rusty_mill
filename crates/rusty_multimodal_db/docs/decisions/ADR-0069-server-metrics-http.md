# ADR-0069: Server Metrics HTTP Endpoint — `GET /metrics` Directly Scrapeable by Prometheus

- Status: **Accepted as designed and implemented** (2026-09-15) —
  option (a), Codex HTTP metrics implementation round. See
  `docs/design/SERVER-METRICS-HTTP-DESIGN.md` for the full design.
- Date: 2026-09-15
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section,
  Metrics/observability bullet (names this exact gap: "Still absent:
  ... any HTTP `/metrics` endpoint"), `ADR-0064`
  (`Request::Metrics`/`Response::Metrics`, protocol 23 — the counter
  set and Prometheus-text rendering this round reuses unchanged; its
  own Considered options already evaluated and declined a standalone
  HTTP listener as option (b), the decision this round revisits),
  `ADR-0010` (the original tokio/HTTP-framework decline this round
  does not reopen), `ADR-0014` (`rusty_tls` reused over a raw
  dependency — the exact ecosystem-reuse template this round follows).
- Supersedes/Superseded by: none proposed. Revisits (does not
  supersede) `ADR-0064`'s own declined option (b) with new information.

## Context

`docs/FUTURE-GROWTH.md`'s Metrics/observability bullet names the gap
directly: *"Still absent: ... any HTTP `/metrics` endpoint — `Metrics`
is answered over the existing binary wire protocol, not scraped by
Prometheus directly."* `ADR-0064` built `Request::Metrics`/
`Response::Metrics` (protocol 23) rendering real Prometheus text
exposition format from a small, fixed atomic counter set — but only
reachable over this crate's own length-prefixed `bincode` wire. A
stock Prometheus install cannot scrape it without a bridge process.

`ADR-0064`'s own Considered options already litigated this exact
shape, as option (b), and declined it: *"Real operational convenience,
at the cost of a second bound port, a new HTTP-server dependency, and
a second transport needing its own security story ... exactly the
kind of new surface `ADR-0043` found this crate never needed for the
client ecosystem question either."* That rejection was written and
accepted 2026-09-14.

**New information found while researching this round, not available
to `ADR-0064`'s own decision**: `Rusty-Mill/rusty_mill`'s own
`crates/rusty_http` — a sans-IO HTTP/1.1 message layer, zero required
dependencies with default features, already in this same workspace —
is structurally the exact `rusty_tls`-for-`rustls` precedent `ADR-0014`
established for TLS. `ADR-0064`'s "a new HTTP-server dependency" cost
premise does not hold against it: a default-features path dependency
on `rusty_http` adds nothing beyond the crate itself, the identical
reuse shape this crate already accepted for `rusty_tls`. (A second,
real external precedent, `tiny_http 0.12`, also exists in the
workspace — `crates/rusty_fedora_agent`'s own dependency, for the
identical "small sync server, no async runtime" case — named as a
documented fallback, not the recommended choice.)

## Decision

**Recommended: option (a)** — a new, opt-in HTTP/1.1 listener, built
on `rusty_http` (path dependency, default features), answering exactly
`GET /metrics` with `ServerMetrics::render()`'s existing text
unchanged, `Content-Type: text/plain; version=0.0.4`, `Connection:
close`. Absent unless an operator configures
`SERVER_METRICS_HTTP_ADDR`; every `ServeOptions` extension since
`ADR-0032` follows this same additive, opt-in shape. Thread-per-
connection over `std::net::TcpListener`, the identical concurrency
model `ADR-0010` already chose — no async runtime, no reopening of
that decision. No authentication or TLS on the new listener in this
round, a named, accepted tradeoff (see Consequences) — matches
Prometheus's own common deployment convention of a private-network-
only scrape target.

Full reasoning, the two alternatives (a bearer-token-gated variant;
declining again), every requirement, and every acceptance criterion
are in `docs/design/SERVER-METRICS-HTTP-DESIGN.md`.

## Consequences

- Positive: closes `docs/FUTURE-GROWTH.md`'s named gap with zero new
  transitive dependency (`rusty_http`, default features, is
  dependency-free — the identical property `rusty_tls` already has)
  and zero change to the existing wire protocol (`PROTOCOL_VERSION`,
  every `Request`/`Response` variant, `SERVER-002` — all untouched; no
  `clients/python/` change either).
- Named, not hidden: the new listener carries no credential check.
  What is disclosed is unchanged (the identical six counters
  `Request::Metrics` already answers to any authenticated wire
  connection, or — on a server with no tokens configured at all — to
  anyone who can already speak the binary wire format at all per
  `AUTH-FR-007`'s existing default). What changes is *reachability*:
  no need to speak the binary protocol anymore. This is a real,
  accepted widening, not an oversight — option (b) in the design
  document is the documented path to a bearer-token gate if the owner
  wants that instead.
- Does not reopen `ADR-0010`'s tokio/async-runtime decision — the new
  listener is synchronous, thread-per-connection, exactly like the
  existing one.
- Absent by default: a server with no `SERVER_METRICS_HTTP_ADDR`
  configured is byte-for-byte unchanged from today.
- This is an additive, opt-in operator capability, sized like
  `ADR-0064`/`ADR-0065` themselves (a bounded new request/listener
  behind a config flag), not `ADR-0010`'s original protocol-defining
  tier — the class of decision `WORKFLOW.md` requires design-first,
  owner-accepted before implementation, which is why this is a
  proposal, not code.

## Considered options

**(a) `rusty_http`-based listener, no endpoint auth** — recommended,
as scoped above; **(b)** same listener, gated by an optional bearer
token (`SERVER_METRICS_HTTP_TOKEN`) checked via `Authorization:
Bearer <token>` — closes the reachability gap (a) accepts, at the
cost of a genuinely new, disconnected-from-the-existing-wire
credential concept, closer to `ADR-0065`/`ADR-0067`'s own heavier
tier; **(c)** decline again, holding `ADR-0064`'s own original call —
the gap stays open, the existing bridge-script workaround
(`ADR-0064`'s own Consequences) remains the answer.

The owner's shorthand: **(a)** the new listener, no endpoint auth, as
proposed; **(b)** same listener, bearer-token-gated; **(c)** decline.

## Acceptance and implementation

- 2026-09-15: proposed, design only.
- 2026-09-15: the owner picked option (a) — the `rusty_http`-based
  listener, no endpoint auth, as recommended. Implementation delegated
  to Codex (`codex-build`), independently inspected by Claude before
  merge.
- 2026-09-15: implemented in the bounded Codex work order; independent
  review pending, not committed or merged. Files: `Cargo.toml`,
  `src/server/metrics_http.rs`, `src/server/serve.rs`,
  `src/server/mod.rs`, `src/bin/dog_server.rs`,
  `src/bin/memory_server.rs`, `src/bin/reminder_server.rs`,
  `src/bin/entity_server.rs`, `tests/server_metrics_http_integration.rs`,
  `docs/design/SERVER-METRICS-HTTP-DESIGN.md`, this
  `docs/decisions/ADR-0069-server-metrics-http.md`, and the workspace
  root `Cargo.lock` (one mechanical dependency-list addition).
  `Cargo.toml` adds `rusty_http = { path = "../rusty_http", optional =
  true }`, with default features, under `server` only — the
  `ADR-0014` ecosystem-reuse precedent, no new transitive dependency.
  `ServeOptions::with_metrics_http` takes an already-bound listener;
  `serve_tables` takes it before `Arc::new(options)` and starts its
  independent accept thread (`MHTTP-FR-001`/`004`). All four binaries
  bind `SERVER_METRICS_HTTP_ADDR` when set, report the configured
  address, and fail at startup if binding fails (`MHTTP-FR-006`).
  `metrics_http.rs` uses `SyncTransport` for one request per connection:
  `GET /metrics` returns the shared `render()` text with exact byte
  length and Prometheus content type, all other methods/paths an empty
  `404`, and bad heads no reply (`MHTTP-FR-002`/`004`). HTTP accepts and
  requests never increment the wire counters; no TLS/auth gate is
  applied to HTTP (`MHTTP-FR-005`, accepted option (a)).
  - Six new unit tests: five in `metrics_http.rs` cover response headers,
    verbatim counter text with bounded uptime, no counter mutation,
    method/path rejection, and malformed/oversized/truncated heads with
    no reply or handler panic; one in `serve.rs` covers the absent-by-
    default listener and opt-in builder (`MHTTP-FR-001`/`002`/`004`).
  - Five new integration tests in `server_metrics_http_integration.rs`
    cover unchanged default wire behavior, live nonzero wire counters
    exposed over HTTP, repeated scrapes that leave counters unchanged,
    empty `404`s, bad heads followed by successful requests on both
    listeners, and fatal invalid-address startup in all four binaries.
    Numeric comparisons with the immediately following wire `Metrics`
    exclude uptime and account for rendering before self-counting.
  - Proof from the workspace root: `cargo fmt -p rusty_multimodal_db --
    --check`, `cargo clippy -p rusty_multimodal_db --all-features --tests
    --bins --lib -- -D warnings`, and `cargo test -p rusty_multimodal_db
    --all-features --no-fail-fast` all passed. Before: 529 library,
    222 integration, 4 binary, and 3 doc tests (758 total). After:
    535 library, 227 integration, 4 binary, and 3 doc tests (769 total),
    no failures or ignored tests; every existing integration target's
    count is unchanged. `cargo tree` confirms only `rusty_http`'s
    default feature, with no new transitive dependency; `Cargo.lock`
    adds only its name to this crate's dependency list.
  - No design corrections or deviations were needed. The explicit
    `ServeOptions::new`/`from_env` constructors initialize the new field
    to `None` as well as the derived `Default`; binary env matches return
    unchanged `options` when unset, matching their existing builder
    chains. Existing integration test files and the wire protocol are
    unchanged. The three open questions are resolved as recommended in
    the design document; cross-project status/roadmap/traceability
    updates remain with the independent reviewer.
