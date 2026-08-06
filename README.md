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
| `agentgateway-mcp` | MCP federation: target connections, name qualification, tool gates. |
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
- CORS, including preflight answered at the gateway
- Graceful shutdown on SIGINT/SIGTERM

Parses but is **not** enforced — reported by `--check` and at startup:

- `ai` backends (the LLM gateway), `a2a` policies, `extAuthz`
- `service` backends (service discovery), `dynamic` backends
- `mcpAuthorization.rules` (upstream's policy-expression form; the
  `allowTools`/`denyTools` lists are ours and *are* enforced)
- TLS termination (`HTTPS`/`TLS` listener protocols)
- `jwtAuth`, `timeout`, `retry`, `localRateLimit`, header modifiers, and
  `urlRewrite` are modelled but not yet applied
- `host` and `service` backends do not proxy yet — an MCP route is the only
  fully served backend kind in v0

Not supported at all:

- The deprecated 2024-11-05 HTTP+SSE target transport (`sse:`). `rmcp` 3.1 has
  no client for it; point the target at the server's Streamable HTTP endpoint
  with `mcp:` instead. Configuring one is a startup error, not a silent skip.

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

**Worth reusing, and not yet done:** its `auth` module is an OAuth 2.1
resource-server layer with JWKS-backed JWT validation, which is most of the
`jwtAuth` policy listed as unimplemented above. `otel` (OTLP spans and metrics)
and `limits` (concurrency and timeout shedding) map onto the observability and
`localRateLimit` gaps. Those are the natural next things to pull in, as a git
dependency pinned to a tag — it is `publish = false`, so there is no crates.io
path.

Its lint posture (`missing_docs = warn`, `unsafe_code = forbid`,
`unwrap_used = warn`) is adopted verbatim in this workspace.

## Tests

```bash
cargo test --workspace
```

49 tests. The config tests are anchored on YAML taken verbatim from
agentgateway's documentation — if upstream examples stop parsing, compatibility
has regressed. The federation tests are genuinely end-to-end: a real `rmcp`
client, over a real socket, through the gateway, into real subprocess MCP
servers (`crates/agentgateway/examples/mock_mcp_server.rs`). Each fixture echoes
its own label in every result, so the tests assert a call reached the *right*
target rather than merely reaching one.

[agentgateway]: https://agentgateway.dev
[rusty_mcp]: https://github.com/baileyrd/rusty_mcp
