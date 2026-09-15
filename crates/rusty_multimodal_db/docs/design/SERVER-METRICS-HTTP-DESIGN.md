# Server Metrics HTTP Endpoint: `GET /metrics` Directly Scrapeable by Prometheus (Proposed)

- Status: **Proposed** (2026-09-15, `ADR-0069`), design only — no source
  file touched.
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section,
  Metrics/observability bullet ("Still absent: ... any HTTP `/metrics`
  endpoint — `Metrics` is answered over the existing binary wire
  protocol, not scraped by Prometheus directly"), `ADR-0064`
  (`SERVER-METRICS-DESIGN.md`, `Request::Metrics`/`Response::Metrics`,
  protocol 23 — the counter set and `render()` this round reuses
  unchanged; its own Considered options already evaluated and declined
  a standalone HTTP listener as option (b), the decision this round
  revisits with new information), `ADR-0010` (the original
  tokio/HTTP-framework decline this round does not reopen), `ADR-0014`
  (`rusty_tls` reused instead of a raw `rustls` dependency — the exact
  ecosystem-reuse template this round follows for HTTP).

## Purpose and scope

`docs/FUTURE-GROWTH.md`'s Metrics/observability bullet names the gap
directly: *"Still absent: ... any HTTP `/metrics` endpoint — `Metrics`
is answered over the existing binary wire protocol, not scraped by
Prometheus directly."* `ADR-0064` built the counter set and its
Prometheus-text rendering (`ServerMetrics::render()`,
`src/server/metrics.rs`) but answers it only over this crate's own
length-prefixed `bincode` wire — a real Prometheus install cannot
scrape it without a bridge process translating one protocol to the
other.

This round proposes closing that gap directly: an opt-in second
listener, on its own configured port, answering a stock Prometheus
scrape (`GET /metrics HTTP/1.1`) with `ServerMetrics::render()`'s
existing text, unchanged, as the response body.

## Non-goals

- **Reopening `ADR-0010`'s tokio/async-runtime decision.** The new
  listener is synchronous, thread-per-connection, over
  `std::net::TcpListener` — the identical model the existing wire
  listener already uses. No async runtime, in this round or as a
  result of it.
- **A general HTTP server, router, or framework.** Exactly one route
  (`GET /metrics`) is recognized; anything else is `404`. No
  middleware, no static files, no JSON API, no admin UI.
- **TLS or credential-gated access on the new listener, in this
  round.** See Decision and Considered options for the full reasoning;
  matches Prometheus's own common deployment convention (a scrape
  target reachable only from a private/internal network, with TLS/auth
  fronted by a reverse proxy when the network isn't trusted) rather
  than reinventing that story inside this crate. Named as a real,
  accepted gap, not hidden — a future round can add an optional bearer
  token exactly the way `ADR-0065`/`ADR-0067` each added a new,
  narrowly-scoped credential when a real write/bulk-read surface
  needed one.
- **Keep-alive, pipelining, chunked request bodies, or any HTTP
  semantics beyond one request/response per connection.** Every
  response carries `Connection: close`; the connection closes right
  after — legal HTTP/1.1 behavior, and the only shape a Prometheus
  scrape (one request, get the body, done) ever needs.
- **A `PROTOCOL_VERSION` bump, a new `Request`/`Response` variant, or
  any change to `SERVER-002` (the wire specification).** This round
  adds a second, independent listener speaking a different, well-known
  protocol (HTTP/1.1) on its own port — the existing binary wire
  protocol, `Request::Metrics` included, is completely untouched.
  `clients/python/` needs no change either.
- **Per-adapter `Unsupported`.** Like `Request::Metrics` itself, the
  new endpoint answers identically regardless of which domain(s) a
  server hosts — it reads `ServeOptions::metrics()`, not any
  `ConnectionStore`.

## Context

Read directly from `src/server/{serve,metrics,mod}.rs`,
`src/bin/{dog,memory,reminder,entity}_server.rs`, `ADR-0010`/`ADR-0014`/
`ADR-0064` and their design docs, the repo-root `Cargo.toml` workspace
member list, and `crates/rusty_http/{Cargo.toml,src/{lib,head,sync,
header,method,status,version}.rs}`, as they stand at `SERVER-001`
v0.56.0:

- **`ServerMetrics::render()` already emits real Prometheus text
  exposition format, byte for byte — zero reformatting needed for an
  HTTP body.** `src/server/metrics.rs`: six `# HELP`/`# TYPE`/sample-line
  triples in one `format!` call (`dogserver_requests_total`,
  `..._ok_total`, `..._err_total`, `dogserver_connections_total`,
  `..._active`, `dogserver_uptime_seconds`). `render(&self) -> String`
  takes `&self`, not `&mut self` — read-only, safe to call from a
  second, independent thread against the same live `ServerMetrics`
  instance the wire-protocol listener is already updating.
- **`ServeOptions::metrics(&self) -> &ServerMetrics` already exists**
  (added by `ADR-0064`'s own implementation correction: `ServerMetrics`
  lives as a plain field on `ServeOptions`, not a separate parameter,
  because `ServeOptions::rate_limit` had already established that
  `ServeOptions` carries live, shared, mutated-per-request state, not
  just configuration). `serve_tables` wraps the whole `ServeOptions` in
  one `Arc` before spawning any connection thread — every thread
  already shares the identical `ServerMetrics` instance by construction.
- **`serve_tables`'s accept loop (`for incoming in listener.incoming()
  { ... }`) never returns in normal operation** — it is the last
  statement of the function and consumes the calling thread until the
  listener errors. A second listener therefore needs its own thread,
  started before that loop is entered.
- **Every `ServeOptions` extension since `ADR-0032` has been additive:
  a new field plus a `with_*` builder method, no `serve`/`serve_tables`
  signature change.** `with_backup_root`, `with_replication_token`,
  `with_tls`, `with_rate_limit` are the precedent. `serve`/
  `serve_tables` never call `TcpListener::bind` themselves for the
  primary listener either — the caller (each `src/bin/*.rs`) binds it
  and handles the bind error its own way; the same posture should hold
  for a second listener.
- **`ADR-0064`'s own Considered options already evaluated and declined
  a standalone HTTP listener, as option (b), citing "a new HTTP-server
  dependency, and a second transport needing its own security story."**
  Quoted exactly: *"Real operational convenience, at the cost of a
  second bound port, a new HTTP-server dependency, and a second
  transport needing its own security story (its own TLS? its own
  auth? shared with `ServeOptions` or not?) — exactly the kind of new
  surface `ADR-0043` found this crate never needed for the client
  ecosystem question either."* That rejection was written and accepted
  2026-09-14 without citing (or, from the record, apparently knowing
  about) either in-workspace precedent this round found — see below.
  This round does not treat that as settled; it revisits the premise
  directly, the same way `ADR-0014` revisited `ADR-0012`'s original
  "no native TLS, too heavy a dependency" call once `rusty_tls` (an
  ecosystem crate built for exactly that reuse) was checked for and
  found.
- **`crates/rusty_http` already exists in the `Rusty-Mill/rusty_mill`
  workspace, and is structurally the exact `rusty_tls`-for-`rustls`
  precedent `ADR-0014` established.** Confirmed by reading its
  `Cargo.toml`/`src/lib.rs` directly: *"One sans-IO HTTP/1.1 message
  layer and `Url` type for the rusty ecosystem."* Zero required
  dependencies with default features (`rusty-tokio`/`tokio`/`cookies`
  are all optional, off by default) — a default-features path
  dependency adds nothing beyond `rusty_http` itself. In scope: request/
  response head parse + serialize, a case-insensitive ordered
  `HeaderMap`, the three body framings, a sync transport adapter over
  any `Read + Write`. Explicitly out of scope (`lib.rs`'s own doc
  comment): *"HTTP/2, TLS ..., compression, multipart, routing
  frameworks"* — no listener/accept-loop/router is provided; this crate
  still writes its own `TcpListener::bind` + accept loop + one-route
  dispatch, exactly as it already does for the binary wire protocol.
  `Cargo.toml`'s own header: *"Per the ecosystem's dependency-
  justification convention (see `rusty_tls`'s Cargo.toml): nothing gets
  added here without a comment justifying it"* — the same convention
  this crate's own `Cargo.toml` already follows for `rusty_tls`.
- **The exact API surface needed already exists.**
  `sync::SyncTransport<T: Read + Write>::new(io)` wraps an accepted
  `TcpStream` directly; `.read_request_head(max_head_len)` returns a
  `RequestHead { method: Method, target: String, version: Version,
  headers: HeaderMap }` (`Method::Get`/`target == "/metrics"` is the
  entire route match this endpoint needs); `.write_response_head(&head)`
  then `.write_body(bytes)` write a `ResponseHead { status: StatusCode,
  reason: String, version: Version, headers: HeaderMap }`.
  `StatusCode::OK`/`StatusCode::NOT_FOUND` are named constants;
  `HeaderMap::insert` sets `Content-Type`/`Content-Length`/`Connection`.
  Nothing here needs hand-writing, unlike `ADR-0014`'s own TLS
  handshake integration, which still had to write `ReadHalf`/`WriteHalf`
  glue around `rusty_tls`'s stream type — this round's HTTP integration
  is a smaller lift than that one was.
- **This crate's own "hand-roll vs. depend" bar, from `pem.rs`'s own
  doc comment, does not clear for HTTP/1.1 parsing the way it did for
  base64.** *"A small, fully-specified, deterministic transform with no
  invisible-to-testing correctness property"* is the stated criterion.
  Real HTTP/1.1 — even restricted to recognizing only `GET /metrics` —
  still has to decide what happens with a pipelined second request, a
  `Connection: keep-alive` header, header folding, or a slow/partial
  head from a hostile or merely buggy client; none of those are "no
  correctness ambiguity" the way a fixed length-prefix
  (`src/server/framing.rs`) or standard base64
  (`src/server/pem.rs`) are. `crates/rusty_http` already exists as the
  answer to exactly this judgment call, so no hand-rolling is even
  needed — this round's design does not ask "hand-roll or depend," it
  asks "reuse the already-built, already-tested ecosystem crate or
  not," the same question `ADR-0014` answered for TLS.
- **One real external-dependency precedent for this exact shape also
  exists in the workspace, as a named alternative:** `crates/
  rusty_fedora_agent` depends directly on `tiny_http = "0.12"`
  (hoisted in the workspace root's `[workspace.dependencies]`), with
  its own Cargo.toml comment reasoning almost identical to this round's
  own: *"no real concurrent I/O to exploit ... deliberately does not
  pull tokio/axum in."* No workspace crate depends on `hyper`/`axum`-
  as-a-listening-server/`actix-web`/`warp`/`rocket` for a bound HTTP
  server; `axum` appears only inside two agent-protocol crates'
  (`rusty_a2a`, `rusty-acp`) own optional server features, unrelated to
  this crate. `rusty_http` is the smaller, more consistent-with-
  `ADR-0010` choice of the two real options: it adds zero new
  dependency at all (an in-workspace sibling crate this repo already
  trusts, matching `rusty_tls`'s own precedent exactly), versus
  `tiny_http` adding one real external crate. `tiny_http` is named here
  as the documented fallback if a future revisit finds `rusty_http`'s
  bare message-layer scope insufficient (e.g. wanting a built-in
  accept-loop/threadpool helper this design hand-writes instead).
- **Every `src/bin/*.rs` server binary shares one `main()` shape**: bind
  the primary `TcpListener` from `argv[1]`, build `ServeOptions` via a
  chain of `std::env::var("SERVER_*")` checks and `with_*` builder
  calls, print one status banner, call `serve`/`serve_tables` once
  (never returning). A `SERVER_METRICS_HTTP_ADDR`-shaped variable would
  need adding identically to all four (`dog_server.rs`,
  `memory_server.rs`, `reminder_server.rs`, `entity_server.rs`) —
  binding a second `TcpListener` and handing it to `ServeOptions` the
  same way `with_backup_root`/`with_tls` already thread their own
  config in.

## Requirements

- `MHTTP-FR-001` **A new, opt-in HTTP/1.1 listener, entirely separate
  from the existing binary wire protocol** — a second `TcpListener`,
  bound by the caller (each `src/bin/*.rs`) exactly the way the
  primary listener already is, handed to `ServeOptions` via a new
  `with_metrics_http(listener: TcpListener)` builder method. Absent by
  default: a server with no `SERVER_METRICS_HTTP_ADDR` configured opens
  no second port and behaves exactly as it does today.
- `MHTTP-FR-002` **Exactly one route: `GET /metrics` answers `200 OK`,
  `Content-Type: text/plain; version=0.0.4`, body
  `ServeOptions::metrics().render()` verbatim** (`ADR-0064`'s existing,
  unchanged Prometheus text). Every other method/path answers `404 Not
  Found` with an empty body. `Connection: close` on every response;
  the connection closes immediately after, no keep-alive.
- `MHTTP-FR-003` **Built on `crates/rusty_http`, a path dependency with
  default features** (no `rusty-tokio`/`tokio`/`cookies`) — zero new
  transitive dependency, matching `ADR-0014`'s own `rusty_tls`-reuse
  template exactly, added under the `server` feature (`client` does
  not need it; only a server binds a listener).
- `MHTTP-FR-004` **Thread-per-connection over `std::net::TcpListener`,
  the identical concurrency model `ADR-0010` already chose** — no
  async runtime, no reopening of that decision. One request read, one
  response written, connection closed; a malformed or oversized head
  drops the connection with no reply and no panic (`SERVER-FR-004`'s
  existing "never panic on a bad client" posture, extended to this
  listener).
- `MHTTP-FR-005` **No authentication or TLS on the new listener in this
  round** — a named, accepted non-goal (see Decision/Considered
  options), not silently assumed away. `Response::Metrics` over the
  existing wire keeps its own existing auth gate untouched; this is a
  genuinely separate, additional way to reach the identical,
  non-sensitive process-health data (request/connection counts,
  uptime) with a lower barrier (no need to speak the binary wire
  protocol at all), explicitly named as the tradeoff being accepted.
- `MHTTP-FR-006` **Every `src/bin/*.rs` server binary gains identical,
  opt-in wiring**: `SERVER_METRICS_HTTP_ADDR` (unset by default),
  parsed and bound the same way the primary listener already is,
  passed to `ServeOptions::with_metrics_http`. A bind failure on this
  second listener is a fatal startup error (`.expect()`/`?`, matching
  how the primary listener's own bind failure is already handled) —
  never silently skipped, so a typo'd address is caught at startup,
  not discovered later as "Prometheus can't reach it."

## Considered options

- **(a) `rusty_http`-based `GET /metrics` listener, opt-in, no
  endpoint auth — recommended, as scoped above.** Closes the named gap
  fully: a stock Prometheus install scrapes this server directly, no
  bridge script. Zero new dependency (`rusty_http` is already an
  in-workspace, zero-required-dependency sibling crate, the same
  `rusty_tls` reuse shape `ADR-0014` already established as this
  ecosystem's own answer to "need a small piece of a heavy protocol's
  functionality"). Does not reopen `ADR-0010`'s tokio decision — both
  listeners stay synchronous, thread-per-connection. Real, named cost:
  the new listener carries no credential check at all, so anyone who
  can reach the configured port sees the same process-health data
  `Request::Metrics` already answers to any authenticated wire
  connection — a real widening of *reachability* (no need to speak the
  binary protocol), not of *what is disclosed* (the identical six
  counters either way). Matches Prometheus's own common deployment
  convention (private-network-only scrape targets, or a reverse proxy
  in front for TLS/auth when the network isn't trusted) rather than
  building a credential story inside this crate for data that was
  never treated as sensitive by `ADR-0064` itself (gated at "any
  authenticated class," the least restrictive gate this crate has).
- **(b) Same as (a), plus an optional bearer token
  (`SERVER_METRICS_HTTP_TOKEN`), checked against the request's
  `Authorization: Bearer <token>` header before rendering; a missing
  or wrong token answers `401 Unauthorized` with no body.** Closes the
  reachability gap (a) accepts. Real, named cost: a genuinely new
  credential concept, disconnected from the existing wire's
  `TokenClass`/`AuthConfig` machinery (HTTP has no session or
  connection-class concept the way the binary protocol's per-connection
  `authenticated` state does) — a small but real new security surface
  to get right (constant-time comparison, matching `AuthConfig::check`'s
  own `subtle`-based precedent, or a fresh timing bug), and a second,
  independently-configured secret an operator now manages alongside
  `SERVER_AUTH_READ_ONLY_TOKEN`/etc. Closer to `ADR-0065`/`ADR-0067`'s
  own heavier "new attack-surface category" tier than (a)'s.
- **(c) Decline again**, holding `ADR-0064`'s own original call.
  `docs/FUTURE-GROWTH.md`'s gap stays open exactly as worded. Zero
  cost, zero risk. A caller needing Prometheus-native scraping today
  already has the workaround `ADR-0064`'s own Consequences named: a
  small bridge process that speaks this crate's existing wire protocol
  (in any language, per `ADR-0043`) and re-serves `Response::Metrics`'s
  text over its own HTTP listener, entirely outside this crate.

The owner's shorthand: **(a)** the new listener, no endpoint auth, as
proposed; **(b)** same listener, bearer-token-gated; **(c)** decline,
the gap stays open.

## Proposed shape

`Cargo.toml`: `rusty_http = { path = "../rusty_http", optional = true
}` under `[dependencies]`, default features (no `rusty-tokio`/`tokio`/
`cookies`) — mirrors `rusty_tls`'s own entry exactly. `server =
["client", "dep:rusty_http"]` (or `client = ["dep:rusty_tls",
"dep:rusty_http"]` if the new listener type needs to be nameable from
a `client`-only build for tests — resolved mechanically during
implementation; the listener itself is only ever *bound* from
`server`-gated code).

`src/server/serve.rs`:

- `ServeOptions` gains one field, `metrics_http: Option<TcpListener>`,
  and one builder method, `with_metrics_http(listener: TcpListener) ->
  Self` — the identical shape `with_backup_root`/`with_tls` already
  use. `#[derive(Default)]` already covers `None`.
- `serve_tables` takes the listener out of `options` (`Option::take`,
  a plain mutable borrow, before wrapping `options` in `Arc`), then, if
  `Some`, spawns one more thread running a new `serve_metrics_http`
  accept loop against `Arc::clone(&options)` — alongside, not instead
  of, the existing wire-protocol accept loop. `serve`/`serve_tables`'s
  own public signatures do not change.
- `serve_metrics_http(listener: TcpListener, options: Arc<ServeOptions>)`:
  the same `for incoming in listener.incoming() { thread::spawn(...) }`
  shape `serve_tables` already uses, calling a new
  `handle_metrics_http_connection` per accepted stream.
- `handle_metrics_http_connection(stream: TcpStream, options:
  &ServeOptions)`: `stream.set_nodelay(true)` (matching
  `handle_connection`'s own precedent); wrap in
  `rusty_http::sync::SyncTransport::new(stream)`;
  `.read_request_head(rusty_http::head::DEFAULT_MAX_HEAD_LEN)` — a
  parse/read failure (malformed head, oversized head, connection
  closed early) drops the connection with no reply, matching
  `SERVER-FR-004`'s existing "never panic on a bad client" posture;
  match `(head.method, head.target.as_str())` against `(Method::Get,
  "/metrics")` — anything else is `404`; on a match, `status =
  StatusCode::OK`, `body = options.metrics().render()`; build a
  `ResponseHead` with `Content-Type`/`Content-Length`/`Connection:
  close` headers and write it plus the body via
  `write_response_head`/`write_body`. Every I/O error past this point
  is discarded (the connection is closing anyway), matching
  `send_response`'s own existing "best effort, no panic" posture on
  the wire-protocol side.

`src/bin/{dog,memory,reminder,entity}_server.rs`: one more
`match std::env::var("SERVER_METRICS_HTTP_ADDR")` arm, identical
shape to the existing `SERVER_BACKUP_ROOT`/`SERVER_TLS_*` chains —
`Ok(addr) => options = options.with_metrics_http(TcpListener::bind(&addr).expect("..."))`,
`Err(_) => {}` (no second listener). One line added to each binary's
existing status banner naming the configured address, matching how
`SERVER_BACKUP_ROOT`/TLS are already announced.

## Data/state and invariants

- The new listener reads `ServeOptions::metrics()` — the identical,
  single `ServerMetrics` instance every wire-protocol connection
  thread already updates via the same `Arc<ServeOptions>` — so an HTTP
  scrape and a concurrent `Request::Metrics` wire call always observe
  the same live counters, never two independently-tracked sets.
  `render(&self)` takes a shared reference; no additional
  synchronization is needed beyond the atomics `ServerMetrics` already
  uses internally.
- The new listener never touches any `ConnectionStore`/domain adapter
  — it is wired at the `ServeOptions`/`serve_tables` layer only, so it
  answers identically regardless of which domain(s) a given server
  binary hosts, the same "no per-domain `Unsupported`" property
  `Request::Metrics` itself already has.
- No new state persists anywhere; `render()` is computed fresh per
  request, exactly as it already is for the wire protocol.

## Errors, failure, recovery, and observability

- A malformed, oversized, or truncated HTTP head: the connection is
  dropped with no reply — `rusty_http`'s own `Error::HeadTooLarge`/
  parse-error/EOF cases all map to "close, no panic," matching
  `handle_connection`'s existing framing-error posture on the binary
  side.
- Any path/method other than `GET /metrics`: `404 Not Found`, empty
  body — never a panic, never a partial/malformed response.
- A bind failure on the configured `SERVER_METRICS_HTTP_ADDR` (bad
  address, port already in use): a fatal startup error in the binary,
  the same posture the primary listener's own bind failure already
  has — never silently skipped.
- No new `ErrorCode`, no wire-protocol change of any kind — this
  listener has nothing to do with `Response::Err`.

## Security, privacy, and compatibility

- **The real, named new surface**: reachability without needing to
  speak the binary wire protocol at all. What is disclosed is
  unchanged — the identical six process-wide counters
  `Request::Metrics` already answers to any authenticated wire
  connection (or, on a server with no tokens configured at all,
  `AUTH-FR-007`'s existing "no tokens configured → `ReadWrite` for
  free" already makes `Request::Metrics` reachable to anyone who can
  open a TCP connection and speak the wire format — a modest,
  effort-based barrier this new listener removes entirely for the
  identical data). Named explicitly, not hidden; option (b) above is
  the documented path to close it if the owner wants that now instead
  of later.
- **Absent by default.** A server with no `SERVER_METRICS_HTTP_ADDR`
  configured opens no second port and is byte-for-byte unchanged from
  today — `AUTH-FR-007`'s own "opt-in never changes default behavior"
  precedent, extended to this listener.
- Does not reopen `ADR-0010`'s tokio/async-runtime decision, and adds
  no new transitive dependency (`rusty_http` with default features is
  dependency-free) — the two properties `ADR-0064`'s own option-(b)
  rejection cited as the cost; both are now available in a way that
  rejection did not have visibility into.
- Wire, hard-to-reverse-once-shipped in the sense of "a running
  deployment now has a second open port if configured," but not a
  `PROTOCOL_VERSION`/`SERVER-002` change at all — sized like
  `ADR-0064`/`ADR-0065` themselves (a bounded, additive operator
  capability), not `ADR-0010`'s original protocol-defining tier.

## Acceptance criteria

1. A server started with no `SERVER_METRICS_HTTP_ADDR` opens exactly
   the one (or, for `memory_server`, the tables' shared) existing
   port(s) — unchanged from today, proven by every pre-existing
   integration test passing unmodified.
2. A server started with `SERVER_METRICS_HTTP_ADDR` set opens a second
   port; `curl`/a raw `TcpStream` sending `GET /metrics HTTP/1.1\r\n
   Host: x\r\n\r\n` receives `200 OK`, `Content-Type: text/plain;
   version=0.0.4`, and a body identical to what `Request::Metrics`
   returns over the existing wire for the same live counters.
3. Any other method or path on the same port answers `404`, never a
   panic, never a hang.
4. A malformed HTTP request (garbage bytes, a head that never
   terminates, a connection closed mid-head) drops the connection with
   no reply and no panic.
5. The existing wire protocol (`PROTOCOL_VERSION`, every `Request`/
   `Response` variant including `Request::Metrics` itself) is
   byte-for-byte unaffected — every existing golden vector and
   integration test passes unmodified, and no `SERVER-002` edit is
   needed.
6. `cargo tree`/`Cargo.lock` show no new transitive dependency beyond
   `rusty_http` itself (default features).

## Verification plan

`cargo test --all-features` (new `src/server/serve.rs` unit/
integration coverage for `handle_metrics_http_connection`'s route
match and error paths; a new `tests/server_metrics_http_integration.rs`
— a real second `TcpListener`, a real HTTP/1.1 request over a raw
`TcpStream`, asserting the response body matches
`Request::Metrics`'s own text for the identical live counters, plus
the 404/malformed-request cases); confirm every existing
`tests/server_metrics_integration.rs` case and every other integration
suite passes unmodified (no wire-protocol change); `cargo tree -p
rusty_multimodal_db --all-features` diffed against the pre-round
lockfile to confirm no unexpected transitive dependency; `cargo fmt`/
`cargo clippy --all-features -- -D warnings` clean.

## Traceability

- Roadmap: `SERVER-METRICS-HTTP-DESIGN` (this document, `Proposed`),
  `SERVER-METRICS-HTTP` (implementation, not started).
- `docs/FUTURE-GROWTH.md`'s Metrics/observability bullet updated once
  implemented — "any HTTP `/metrics` endpoint" moves from "still
  absent" to named, bounded, and built, the identical treatment every
  other "Operational maturity" item already received.

## Open questions

- **Should `server`'s feature gate list `rusty_http` directly, or
  should `client` gain it too** (in case a future round wants an HTTP
  client-side capability, e.g. a health-check helper)? Recommendation:
  `server`-only for this round — nothing client-side needs `rusty_http`
  yet, and adding it to `client` unconditionally would grow the
  `client`-alone build's dependency footprint for a capability that
  does not exist yet (`ECO-FR-001`'s own "the client half only" scoping
  precedent).
- **Should `memory_server.rs` (the one binary using `serve_tables`
  directly, with more than one table) get its own distinct env var, or
  share the exact same `SERVER_METRICS_HTTP_ADDR` name every other
  binary uses?** Recommendation: the same name — the metrics endpoint
  is process-wide, not per-table, matching `ServerMetrics` itself being
  one instance per `serve_tables` call regardless of table count.
- **Is a `404` body worth a small, fixed plain-text message (e.g. "not
  found") instead of empty**, for a human accidentally hitting the
  wrong path in a browser? Recommendation: empty is fine — this
  endpoint's only real client is Prometheus's own scraper, which never
  renders a body on a non-2xx response; a future round can add a
  friendlier message if a real operator asks for one. Left for the
  implementation to resolve — a mechanical detail, not a design fork
  the owner needs to weigh.
