# rusty_agent_gateway

A Rust implementation of the [agentgateway] data plane, built to be a drop-in
for its configuration file: an existing `config.yaml` should parse and serve
unmodified.

This is v0. It covers the shared foundation — configuration, listeners, route
matching, policies — plus the **MCP gateway**: several upstream MCP servers
federated into one endpoint, with tool-level filtering and authorization. The
A2A and LLM pillars are not built yet; see [Scope](#scope).

```bash
cargo run -p agentgateway -- --file examples/mcp-federation.yaml
```

## What it does

```yaml
binds:
  - port: 3000
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              cors:
                allowOrigins: ["*"]
                exposeHeaders: ["Mcp-Session-Id"]
            backends:
              - mcp:
                  targets:
                    - name: everything
                      stdio:
                        cmd: npx
                        args: ["@modelcontextprotocol/server-everything"]
                    - name: fetch
                      stdio:
                        cmd: uvx
                        args: ["mcp-server-fetch"]
```

One MCP endpoint on `:3000/mcp`, backed by two subprocess servers. Their tools
are unioned and qualified by target, so both can export `search` without
colliding:

```
$ curl … -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
everything_echo, everything_add, …, fetch_fetch, …
```

A call to `fetch_fetch` is routed to the `fetch` target and forwarded with the
tool's own name.

## Architecture

| Crate | Responsibility |
| --- | --- |
| `agentgateway-config` | The configuration model. Wire-compatible with agentgateway's local config. |
| `agentgateway-core` | Route matching (Gateway API precedence), hostname patterns, CORS. |
| `agentgateway-auth` | The `jwtAuth` policy, over [`rusty_mcp`][rusty_mcp]'s JWKS validator. |
| `agentgateway-mcp` | MCP federation: target connections, name qualification, tool gates. |
| `agentgateway-proxy` | HTTP reverse proxying for `host` backends. |
| `agentgateway-tls` | TLS termination, over [`rusty_tls`][rusty_tls]. |
| `agentgateway` | The binary: data plane assembly, sockets, graceful shutdown. |

Three decisions are worth knowing up front, because they shape everything else.

**Unknown config fields are accepted, not rejected.** Upstream ships fields
faster than we implement them, and refusing to boot on one we do not support
yet would defeat the point of being a drop-in. `Config::lint` reports what
parsed but is not acted on, and the binary logs each finding at startup — so
the tolerance stays visible rather than silent. A policy that looks enforced
but is not is worse than one that fails loudly, because it looks like security.

**Route precedence follows Gateway API, not file order.** An exact path beats a
prefix, a longer prefix beats a shorter one, then method, header and query
predicates break ties. Prefix matching is segment-aware: `/admin` does not
capture `/admin-public`.

**Tool gates are enforced on `tools/call`, not just `tools/list`.** Nothing
stops a client from calling a name it was never shown. A tool hidden from the
catalogue but still callable is worse than one that was never hidden, because
the operator believes it is gone. There is a test for exactly this.

### Federated tool names

Two servers behind one endpoint may both export `search`, so names are
qualified: `github_search`, `jira_search`.

Resolving that back is sharper than it looks. Splitting on the first `_` breaks
the moment a target is called `code_search` — its `index` tool federates to
`code_search_index` and splits back to target `code`, tool `search_index`,
routing the call to the wrong server. So resolution matches against the known
target names, longest first. Genuinely ambiguous setups still exist, and
`ToolNamer::collisions` reports them at startup rather than letting the gateway
silently pick one.

Set `nameMode: passthrough` on the backend to expose names unchanged; a
collision is then reported as a startup warning.

### Degraded operation

A target that fails to come up does not take the gateway down. Five targets
behind one endpoint means five things that can be broken at any moment, and
refusing to serve the four healthy ones because the fifth is restarting is not
the trade a gateway should make. Failures are logged at startup and reported by
`Federation::degraded`; only losing *every* target is fatal.

## Scope

Implemented and tested:

- `binds` / `listeners` / `routes` / `backends` / `policies`
- Route matching: path (exact, segment-aware prefix, regex), method, headers,
  query — with percent-decoding, and Gateway API precedence
- Hostname matching on listeners and routes, including single-label wildcards
- `mcp` backends: `stdio` and Streamable HTTP (`mcp:`) targets, federation, tool
  name qualification, per-target `filters`, route-level `mcpAuthorization`
  (`allowTools` / `denyTools`)
- `host` backends: HTTP reverse proxying with weighted load balancing,
  `urlRewrite`, header modifiers and `backendAuth` — see [Proxying](#proxying)
- `retry` with backoff, and `localRateLimit` token buckets — see
  [Retries and rate limits](#retries-and-rate-limits)
- TLS termination with ALPN (`h2` and `http/1.1`) — see [TLS](#tls)
- CORS, including preflight answered at the gateway
- `jwtAuth`: JWKS-backed JWT validation (`url:` or `file:`), issuer and audience
  binding, RFC 6750 `WWW-Authenticate` challenges
- `timeout`: both `requestTimeout` and `backendRequestTimeout` — see
  [Timeouts](#timeouts), because they bound different things and only one of
  them bounds a tool call
- OpenTelemetry: OTLP traces and metrics, with MCP request metrics labelled by
  method and tool name
- Process-wide load shedding: a concurrency bound answered with `503` and
  `Retry-After`
- Graceful shutdown on SIGINT/SIGTERM

Parses but is **not** enforced — reported by `--check` and at startup:

- `ai` backends (the LLM gateway), `a2a` policies, `extAuthz`
- `service` backends (service discovery), `dynamic` backends
- `mcpAuthorization.rules` (upstream's policy-expression form; the
  `allowTools`/`denyTools` lists are ours and *are* enforced)
- SNI: one certificate per port. Two listeners on one port with different
  certificates is a startup error rather than a guess
- `protocol: TLS` (opaque passthrough) is terminated as HTTPS rather than
  forwarded
- `retry`, `localRateLimit`, header modifiers and `urlRewrite` are modelled but
  not yet applied
- `service` backends need service discovery; use `host` with a literal address

Not supported at all:

- The deprecated 2024-11-05 HTTP+SSE target transport (`sse:`). `rmcp` 3.1 has
  no client for it; point the target at the server's Streamable HTTP endpoint
  with `mcp:` instead. Configuring one is a startup error, not a silent skip.

## Authentication

A route with a `jwtAuth` policy requires a bearer token:

```yaml
policies:
  jwtAuth:
    issuer: https://auth.example.com
    audiences: ["https://gateway.example.com/mcp"]
    jwks:
      url: https://auth.example.com/.well-known/jwks.json
```

Three details are worth knowing, each with a test named after it.

**The audience check is ours, and it is the point.** [`rusty_mcp`][rusty_mcp]'s
`JwtValidator` reads `aud` and deliberately checks nothing, because upstream's
own layer binds it to a single canonical resource URI while our `audiences` is a
list. Skipping it would be the confused-deputy hole: a caller replays a token
minted for some other service and borrows this gateway's privileges. An empty
`audiences` accepts any audience — a documented opt-out for deployments that
bind audience upstream, not an oversight.

**Authentication runs after the CORS preflight branch.** Browsers never send
`Authorization` on a preflight, so requiring a token there would make every
cross-origin call fail before the real request was ever sent. Rejections still
carry CORS headers, because a 401 the browser cannot read is a 401 nobody can
act on.

**A validator outage is a 503, never a 401.** If the JWKS endpoint is
unreachable the client's token may be perfectly good; answering 401 would send a
user through a login that fixes nothing and disguise an outage as an auth
problem.

A `file:` JWKS is read once at startup, so a missing, malformed or empty key set
stops the gateway booting instead of turning every request into a runtime error
that reads like a client problem. Rotating that file needs a restart — which is
what the `url:` form is for.

## Proxying

A `host` backend forwards bytes rather than terminating a protocol, which makes
it where the policies that only make sense for a proxy finally do something:

```yaml
- name: api
  matches:
    - path:
        pathPrefix: /api
  policies:
    urlRewrite:
      path:
        prefix: /v1          # /api/things -> /v1/things
    requestHeaderModifier:
      set: {x-gateway: rusty}
    backendAuth:
      key: backend-secret
  backends:
    - host: "10.0.0.1:8080"
      weight: 3
    - host: "10.0.0.2:8080"
      weight: 1
```

Weighted selection is deterministic round-robin over a weighted ring, not
random choice: randomness only reaches the configured ratio in expectation, so
a low-traffic route can sit lopsided for a long time. Weight `0` drains a
backend without deleting its configuration; every weight being `0` is a startup
error, since the route could never send traffic anywhere.

Hop-by-hop headers are stripped in both directions — **including the headers
the `Connection` header names**, which is the half that is easy to miss:
`Connection: x-custom` makes `x-custom` hop-by-hop for that message, and a
proxy that only removes the fixed RFC list happily forwards it.

The client address is appended to `X-Forwarded-For` rather than replacing it,
because overwriting the chain erases every proxy before us. The upstream's
`Host` is rewritten to name the upstream, or a name-based virtual host serves
the wrong site. `backendAuth: key` replaces the client's own `Authorization`
rather than adding to it, so a client cannot smuggle its credential to a
backend that should see only ours.

An unreachable upstream is a `502`, not a `500`: the gateway is fine, the
upstream is not, and conflating the two sends people to debug the wrong
process. Unlike MCP, a proxied response is not produced until the upstream
answers, so `backendRequestTimeout` genuinely bounds the wait here.

A route mixing `host` with a backend kind this build cannot serve is refused
rather than served: silently dropping the unsupported share onto the hosts
would send traffic somewhere the operator never asked for.

## TLS

```yaml
binds:
  - port: 8443
    listeners:
      - protocol: HTTPS
        tls:
          cert: /etc/certs/tls.crt   # chain, leaf first
          key: /etc/certs/tls.key    # PKCS#8, PKCS#1 or SEC1
        routes: [...]
```

Termination goes through [`rusty_tls`][rusty_tls], the ecosystem's one TLS
implementation, so the gateway does not roll its own. ALPN advertises `h2` and
`http/1.1`; over TLS that is how the HTTP version gets chosen, and a client
capable of HTTP/2 gets it.

Certificates are read at startup, so a missing or malformed one stops the
gateway booting instead of failing every handshake later. `X-Forwarded-Proto`
reports `https` for a TLS listener — an upstream generating absolute URLs from
that header would otherwise emit `http://` links into an `https://` page and
trip mixed-content blocking.

**One certificate per port.** `rusty_tls` builds its acceptor from a single
chain and does not surface `rustls`' `ResolvesServerCert`, so SNI-based
selection is not available. Two listeners on one port with different
certificates is refused at startup rather than quietly serving the first one's
certificate to the second one's clients.

## Retries and rate limits

```yaml
policies:
  retry:
    attempts: 2          # retries *after* the first try, so three tries total
    backoff: 100ms       # doubles each attempt, capped at 30s
    codes: [502, 503]
  localRateLimit:
    - maxTokens: 100     # burst
      tokensPerFill: 100
      fillInterval: 60s
    - maxTokens: 5       # sustained
      tokensPerFill: 5
      fillInterval: 1s
```

### What is safe to retry

A **connect** failure never reached the upstream, so replaying it cannot
duplicate work. Any other transport error is ambiguous — the request may have
arrived and been processed, with only the response lost on the way back — so it
is **not** retried. Replaying that would silently double a payment or a write.
A timeout is ambiguous for the same reason and is not retried either.

A **status code** is different: the upstream answered, so it certainly saw the
request. Retrying is the operator's explicit choice by listing the code, which
is why nothing is retried on status unless `codes` names it. Each attempt takes
the next endpoint in the ring, since retrying the instance that just failed is
the least likely way to succeed.

### Why a body has to be buffered

A retry replays the request, and a streaming body can only be read once. A body
is buffered only when its size is **known in advance** and fits in 64 KiB — so
the body is never partially consumed to find out how big it is, which would
leave a stream that can be neither replayed nor forwarded. Requests with a
`Content-Length` inside the limit get retries; chunked or oversized ones are
streamed straight through and simply do not.

### Rate limits

Several limits on one route must *all* permit a request, which is how burst and
sustained rates are expressed together. They share one lock: checking them in
turn would spend a token from the burst bucket before discovering the sustained
one refuses, billing a request that was never served.

Refill is lazy — no background task ticks every bucket in the process — and
credits whole elapsed intervals only, because advancing to *now* on each check
would discard the remainder every time and refill slower than configured.
Buckets start full, so a gateway that has just restarted does not spend its
first interval refusing traffic.

Over the limit is `429` with `Retry-After`, reporting the *longest* wait across
the limits that refused: coming back when only the shorter one has refilled
would just be refused again. `Retry-After` is never `0`, which would invite an
immediate retry.

Rate limiting runs **before** authentication, so a flood is refused before it
costs a signature verification and possibly a JWKS fetch. It runs after the
CORS preflight branch for the same reason authentication does — a browser
reports a refused preflight as an opaque CORS error, hiding the 429 the caller
needs to see.

## Timeouts

The two budgets in a `timeout` policy bound genuinely different things, and the
difference is not cosmetic:

```yaml
policies:
  timeout:
    requestTimeout: 30s          # time to *produce* a response
    backendRequestTimeout: 10s   # time for an upstream call to finish
```

Measured against a tool that sleeps five seconds, `time_starttransfer` is ~1ms
and `time_total` is ~5s: the Streamable HTTP transport sends its SSE response
headers immediately and streams the JSON-RPC result afterwards. So by the time
a tool starts running, `requestTimeout` has already been satisfied — **it
cannot cut a tool call off, and a route that sets only `requestTimeout` has no
bound on how long a tool may run.**

That is deliberate rather than a defect: bounding the whole stream would sever
every long-lived subscription. `backendRequestTimeout` is the budget that
bounds a tool call, applied around the upstream request inside the federation.
A call that exceeds it comes back as a tool error naming the tool and the
budget — a result the model can read, not an opaque protocol failure. It also
bounds `tools/list`, so one hung target cannot hold up the whole catalogue.

There are tests pinning both halves, including one asserting that
`requestTimeout` does *not* kill a long stream, so nobody "fixes" it into
something that severs live subscriptions.

## Observability

```yaml
config:
  tracing:
    endpoint: http://localhost:4317
    serviceName: rusty-agent-gateway
    sampleRatio: 1.0
  limits:
    maxConcurrentRequests: 256
    requestTimeout: 30s
```

With `tracing` set, spans and metrics go to an OTLP collector. Sampling is
parent-based: a caller that already sampled the trace is followed, because
deciding independently is how traces end up half-recorded with gaps exactly
where a service made its own choice.

MCP request metrics are labelled from the `Mcp-Method` and `Mcp-Name` headers,
so no request body is parsed. The tool-name label is bounded by the names each
route actually federates — anything else is labelled `other`. Without that
bound, a client could mint unbounded time series by calling names that do not
exist, which is how a metrics backend gets taken down from the outside.

`maxConcurrentRequests` sheds with `503` and `Retry-After` rather than queueing:
a queue in front of an overloaded gateway turns a capacity problem into a
latency problem, where every client waits longer, times out and retries — which
is how a brief spike becomes a sustained outage. It counts requests until a
response is *produced*, so as above it bounds handshakes, authentication and
JWKS fetches rather than streaming tool calls. That is the load worth shedding:
an unauthenticated flood costs a token validation each.

Both limits are off unless configured. There is no concurrency number right for
everyone, and a default would be a silent regression for a gateway already
serving more than it.

## On `rusty_mcp`

[`baileyrd/rusty_mcp`][rusty_mcp] was evaluated for reuse here. It is a
server-side scaffold on `rmcp` 3.x, and the overlap is real but partial.

**Not reused: its `runtime` and `cli` layer.** `rusty_mcp::serve` binds a socket
and owns one server on one transport at one path. A gateway is
`binds[] → listeners[] → routes[]`, with MCP as one backend kind among several,
so it needs a mountable `tower` service rather than something that binds for
you — which is `rmcp`'s `StreamableHttpService` directly, and what this repo
uses.

The deeper mismatch is protocol version. `rusty_mcp` pins **2026-07-28** and
defaults to `NeverSessionManager`, deliberately: that revision is stateless, and
the payoff is horizontal scaling with no session affinity. A gateway cannot take
that position. It has to speak whatever its clients speak and whatever each
target speaks, and today's ecosystem is largely 2025-06-18/2025-11-25 with
sessions — which is why upstream's own quickstart puts `Mcp-Session-Id` in
`exposeHeaders`, and why this gateway uses `LocalSessionManager`.

**Reused: its `auth` module**, as a git dependency pinned to `v0.4.1` (it is
`publish = false`, so there is no crates.io path). `JwtValidator` brings JWKS
caching with a TTL, an algorithm allow-list pinned *before* any key is loaded —
which is what defeats the `alg: none` and RS256-verified-as-HS256 attacks — and
a floor between refetches provoked by an unknown `kid`, without which anyone can
force unbounded outbound requests to the authorization server by presenting
tokens with random `kid` values.

Two things it does not cover, supplied by `agentgateway-auth`: the audience
check described above, and a file-backed JWKS, since `JwtValidator` fetches over
HTTP only. `FileJwks` mirrors its verification path deliberately, including the
ordering that matters. If `rusty_mcp` grows a `JwtValidator::from_jwks`
constructor, that module should collapse into it — duplicated crypto is
duplicated risk, and this is the one place this repo carries any.

**Also reused: `otel` and `limits`.** `otel::init` stands up the OTLP pipeline
and installs the subscriber; `OtelGuard` is held until after the accept loops
stop, because spans are batched and whatever is still buffered dies with the
process otherwise. `McpMetricsLayer` supplies the MCP request metrics described
above. `LimitsLayer` supplies the concurrency bound, mounted outside routing and
authentication so a shed request costs a semaphore try-acquire rather than a
route lookup, a token validation and a JWKS fetch.

One correction worth recording: `limits` is concurrency and timeout shedding,
**not** a token bucket, so it does not cover `localRateLimit`
(`maxTokens`/`tokensPerFill`/`fillInterval`). That policy remains unimplemented
and needs its own bucket.

Its lint posture (`missing_docs = warn`, `unsafe_code = forbid`,
`unwrap_used = warn`) is adopted verbatim in this workspace.

## On `rusty_tls`

TLS termination is [`rusty_tls`][rusty_tls] — a `rustls` 0.23 wrapper — rather
than `tokio-rustls`, so the gateway is not the one consumer in this ecosystem
that rolls its own TLS.

Its async server adapter is written against `rusty_tokio`'s `AsyncRead` and
`AsyncWrite`, while this gateway runs on `tokio` and hands connections to
`hyper`, so a TLS stream is adapted twice: the socket goes in as a
`rusty_tokio` stream and the decrypted stream comes back out as a `tokio` one.
**That costs nothing per byte.** Both runtimes' `ReadBuf` is an initialized
`&mut [u8]` plus a filled cursor, so the inner reader writes directly into the
outer buffer's spare capacity and the adapter only forwards the count — no
intermediate buffer, no copy, no `unsafe`. The price of the choice is
dependency weight (`rusty_tokio` for its trait definitions, and `rustils`'
`platform` crates that only client-side trust anchors use), not throughput.

One deliberate exception to the crate's "consumers import `rusty_tls`, never
`rustls`" rule: `rusty_tls` does not install a `rustls` `CryptoProvider`, and
`rustls` refuses to guess when more than one is present in a build. This
workspace has two — `ring` via `rusty_tls`, `aws-lc-rs` via `reqwest` — so it
panics on the first handshake. `agentgateway-tls` installs `ring` explicitly,
once, which needs a direct `rustls` dependency pinned to the same 0.23.

## Tests

```bash
cargo test --workspace
```

155 tests. The config tests are anchored on YAML taken verbatim from
agentgateway's documentation — if upstream examples stop parsing, compatibility
has regressed. The federation tests are genuinely end-to-end: a real `rmcp`
client, over a real socket, through the gateway, into real subprocess MCP
servers (`crates/agentgateway/examples/mock_mcp_server.rs`). Each fixture echoes
its own label in every result, so the tests assert a call reached the *right*
target rather than merely reaching one.

The JWT tests generate an RSA keypair at test time and sign real tokens, so
signatures are genuinely verified rather than stubbed — including a forged token
signed by an untrusted key and an `alg: none` token.

The timeout tests drive a genuinely slow subprocess rather than a stubbed
future, which is how the `requestTimeout`/`backendRequestTimeout` distinction
above was found rather than assumed.

The proxy tests stand up a real upstream that echoes back the request line and
headers it saw, so the assertions are about what actually arrived rather than
what the gateway believed it sent. The retry tests script that upstream — "fail
twice, then succeed" — and count hits, which is how "did not retry" is told
apart from "retried and got the same answer".

The rate limiter takes time as a parameter rather than reading a clock, so its
tests drive time instead of sleeping. A rate limiter tested with `sleep` is one
tested at a single resolution, on one machine, when CI was not busy.

[agentgateway]: https://agentgateway.dev
[rusty_mcp]: https://github.com/baileyrd/rusty_mcp
[rusty_tls]: https://github.com/baileyrd/rusty_tls
