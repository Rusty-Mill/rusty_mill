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
| `agentgateway-auth` | The `jwtAuth` and `extAuthz` policies. |
| `agentgateway-mcp` | MCP federation: targets, tools, prompts, resources, gates, CEL rules, guardrails. |
| `agentgateway-proxy` | HTTP reverse proxying for `host` backends. |
| `agentgateway-tls` | TLS termination, over [`rusty_tls`][rusty_tls]. |
| `agentgateway-llm` | The LLM gateway: an OpenAI-compatible front end over providers. |
| `agentgateway-a2a` | A2A method gating and agent-card discovery, over [`rusty_a2a`][rusty_a2a]. |
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

### Federated names and URIs

Two servers behind one endpoint may both export `search`, so names are
qualified: `github_search`, `jira_search`. Prompts take the same treatment.

Resolving that back is sharper than it looks. Splitting on the first `_` breaks
the moment a target is called `code_search` — its `index` tool federates to
`code_search_index` and splits back to target `code`, tool `search_index`,
routing the call to the wrong server. So resolution matches against the known
target names, longest first. Genuinely ambiguous setups still exist, and
`ToolNamer::collisions` reports them at startup rather than letting the gateway
silently pick one.

Resources cannot: they are identified by URI, and `alpha_memo:insights` is not
a URI any client would accept. Upstream's answer, which this follows, is to
widen the scheme instead — `memo:insights` on target `alpha` becomes
`alpha+memo:insights`, still a well-formed URI because `+` is legal in a
scheme. A URI with no scheme is left alone; there is nothing to widen, and
inventing one would change what the client asked for. Contents come back from
the upstream carrying its own URIs, so they are re-qualified on the way out —
otherwise no client could read the URI back to us.

Set `nameMode: passthrough` on the backend to expose names and URIs unchanged;
a collision is then reported as a startup warning.

Prompt and resource capabilities are advertised only when some target actually
has them, read from what each server sent in its handshake. Claiming prompts
the federation cannot serve would have clients call `prompts/list` and be told
the method does not exist. A target that never advertised prompts is not asked
for them either — a missing capability is not a fault, and a mixed federation is
the normal case.

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
- `mcp` backends: `stdio` and Streamable HTTP (`mcp:`) targets, federating
  tools, prompts and resources with name and URI qualification, per-target
  `filters`, route-level `mcpAuthorization` — both the `allowTools`/`denyTools`
  lists and upstream's CEL `rules`, see [Authorization](#authorization)
- `mcpGuardrails`: external MCP policy processors over gRPC, able to rewrite as
  well as refuse — see [Guardrails](#guardrails)
- `host` backends: HTTP reverse proxying with weighted load balancing,
  `urlRewrite`, header modifiers and `backendAuth` — see [Proxying](#proxying)
- `retry` with backoff, and `localRateLimit` token buckets — see
  [Retries and rate limits](#retries-and-rate-limits)
- `ai` backends: an OpenAI-compatible API over OpenAI and Anthropic, streaming
  included — see [The LLM gateway](#the-llm-gateway)
- `a2a` policies: JSON-RPC method gating and a merged agent card — see
  [Agent-to-agent](#agent-to-agent)
- TLS termination with ALPN (`h2` and `http/1.1`) — see [TLS](#tls)
- CORS, including preflight answered at the gateway
- `jwtAuth`: JWKS-backed JWT validation (`url:` or `file:`), issuer and audience
  binding, RFC 6750 `WWW-Authenticate` challenges
- `extAuthz`: an external authorization service consulted before a request is
  served, failing closed — see [External authorization](#external-authorization)
- `timeout`: both `requestTimeout` and `backendRequestTimeout` — see
  [Timeouts](#timeouts), because they bound different things and only one of
  them bounds a tool call
- OpenTelemetry: OTLP traces and metrics, with MCP request metrics labelled by
  method and tool name
- Process-wide load shedding: a concurrency bound answered with `503` and
  `Retry-After`
- Graceful shutdown on SIGINT/SIGTERM

Parses but is **not** enforced — reported by `--check` and at startup:

- `extAuthz.includeBody`: the authorizer sees the method, path and allow-listed
  headers, never the body
- `mcpGuardrails` processors naming `backend:` or `service:` rather than
  `host:`
- `service` backends (service discovery), `dynamic` backends
- SNI: one certificate per port. Two listeners on one port with different
  certificates is a startup error rather than a guess
- `protocol: TLS` (opaque passthrough) is terminated as HTTPS rather than
  forwarded

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

## External authorization

An `extAuthz` policy asks an authorization service about each request before it
is served:

```yaml
policies:
  extAuthz:
    target: http://authz.internal:9000
    timeout: 250ms
    includeHeaders: [authorization]
    allowedUpstreamHeaders: [x-user-id]
```

The call carries the request's original method and path, so the authorizer can
route on what is being authorized instead of reading it back out of a header. A
2xx allows the request; anything else denies it, and the authorizer's own
status, body and `WWW-Authenticate` are returned to the caller — an authorizer
answering `403 {"reason": "not in group"}` is telling the caller something a
generic "forbidden" would throw away.

**It fails closed.** When the authorizer cannot be reached the request is
denied, because an authorization service that is down must not become an open
door. `failOpen: true` reverses that for deployments that would rather serve
than stall, but it has to be asked for. The refusal is a **503, not a 403**:
nothing decided the request was forbidden, and saying "forbidden" would send
someone to check their permissions when the real problem is a service being
down.

**Both header lists are allow-lists, and the outbound one matters more.**
`includeHeaders` bounds what the authorizer sees, keeping cookies and payloads
away from a service that has no need for them. `allowedUpstreamHeaders` bounds
what the authorizer may set on the request that continues upstream — without
it, an authorizer could write any header the upstream trusts (`x-user-id`,
`x-is-admin`), which turns an authorization service into an impersonation
service. Both default to empty, which allows nothing rather than everything.

The default budget is 250ms. This call sits in front of every request on the
route, so a slow authorizer is a slow gateway; exceeding the budget is an
unreachable authorizer and takes the fail-closed path.

`extAuthz` runs after `jwtAuth`, so an authorizer configured with
`includeHeaders: [authorization]` sees a token the gateway has already
validated. `includeBody` parses but is not enforced — the authorizer never sees
a request body, so a policy that depends on one will not do what it says.

## Authorization

An `mcpAuthorization` policy decides what a caller on the route may reach.
Two forms, and they compose:

```yaml
policies:
  mcpAuthorization:
    # Regexes over the federated tool name. Tools only.
    denyTools: ["_delete$"]
    # CEL expressions over the call and the caller.
    rules:
      - 'mcp.tool.name == "echo"'
      - 'jwt.sub == "test-user" && mcp.tool.name == "get-sum"'
      - 'mcp.prompt.name == "summarize"'
      - 'mcp.resource.name.startsWith("memo:")'
      - require: 'jwt.iss == "https://auth.example.com"'
```

`allowTools`/`denyTools` are name matching and nothing else, and — as the name
says — they are about tools. `rules` cover **tools, prompts and resources**, and
are what you need when the answer depends on *who* is asking: `jwt` is the
verified token's claims, so a rule can grant one thing to one subject and
another to everyone.

Every gate is enforced on the call, not only on the listing — `tools/call`,
`prompts/get` and `resources/read` each re-check. Filtering the catalogue alone
is security theatre: nothing stops a client asking for a name it was never
shown, and something hidden from the listing but still reachable is worse than
something that was never hidden, because the operator believes it is gone.

### One subject is bound per call

Exactly one of `mcp.tool`, `mcp.prompt` and `mcp.resource` exists for any one
call. On a `prompts/get`, `mcp.tool.name` does not resolve, so
`mcp.tool.name == "echo"` reads as **false** — not as "this rule isn't about
prompts, skip it".

That has a consequence worth stating plainly, because it is the one that
catches people:

> **A rule set written entirely as tool `allow` rules refuses every prompt and
> resource.** It is an allow-list, and nothing in the prompt or resource space
> can satisfy it.

This is the safe direction — a policy that had never heard of prompts should not
wave them through — and it is upstream's behaviour. But it does mean that adding
prompts to a target behind an existing tool allow-list takes an explicit rule.
A pure `deny` set behaves the opposite way: it names what is refused, so a
prompt it does not name survives.

### A rule sees the unqualified name

`mcp.tool.name` is the tool's own name on its target, and `mcp.tool.target` is
the target — the pair *before* federation joins them. A tool federated as
`everything_echo` is `mcp.tool.name == "echo"` with `mcp.tool.target ==
"everything"`, so a rule written against `everything_echo` never fires. Prompts
work identically, and a resource's `mcp.resource.name` is the target's own URI:
`memo:insights`, not the federated `alpha+memo:insights`.

(`mcp.resource.name`, not `.uri` — upstream names the field for the shape it
shares with tools and prompts rather than for what a resource calls its
identifier.)

That is upstream's split, and it lets a rule name something without knowing what
the gateway will prefix it with. It is also the opposite of
`allowTools`/`denyTools`, which match the federated name — which is what lets
one route ban a tool on one target while leaving the same name on another
alone. Both readings are right for their own form; the difference is worth
knowing because getting it wrong makes a policy silently not apply.

### Precedence

1. No rules at all permits, so adding an empty `rules:` list does not take the
   route offline.
2. Any `deny` that holds refuses, ahead of everything else.
3. Every `require` must hold.
4. Any `allow` that holds permits.
5. Otherwise: permitted only if there were no `allow` rules. A set of pure
   `deny` rules is a deny-list, so what it does not name survives; the moment
   one `allow` exists the set becomes an allow-list.

A bare string is an `allow`, which is how upstream's examples are written.

### Prefer `require` to `deny`

An expression that cannot be evaluated — no `jwt` on a route without `jwtAuth`,
a claim that is not there, a type mismatch — counts as **false**. That is
upstream's behaviour, and it is the safe direction for `allow` and `require`,
which fail towards refusing.

It is the wrong direction for `deny`: a `deny` that errors permits the call.
So `deny: 'jwt.role == "banned"'` on a request with no token lets the request
through, where `require: 'jwt.role != "banned"'` refuses it. Both are
supported, and there is a test for each pinning the difference down.

An expression that does not compile is a startup failure, not a skipped rule —
otherwise a typo in an `allow` silently serves nothing and a typo in a `deny`
silently refuses nothing.

## Guardrails

An `mcpGuardrails` processor is an MCP-aware policy service the gateway
consults over gRPC — Envoy's `ext_authz` shape moved down to the MCP method
layer, with one addition that changes what it is for: **a processor can rewrite
as well as refuse.** Redacting a secret out of a tool result is not something a
yes/no answer can do.

```yaml
policies:
  mcpGuardrails:
    processors:
      - kind: remote
        host: guardrail.internal:9000
        timeout: 5s
        methods:
          tools/call: full        # both phases
          "prompts/*": full       # a whole namespace
          "*/list": response      # results only
        failureMode: failClosed   # the default
        metadata:
          tenant: 'request.headers["x-tenant"]'
        requestHeaders:
          allowed: [x-tenant]
```

The wire protocol is upstream's `agentgateway.dev.ext_mcp` — `CheckRequest` and
`CheckResponse`, each answering pass, a replacement body, or a refusal.
`proto/ext_mcp.proto` in `agentgateway-mcp` is upstream's schema, unchanged. The
messages are written by hand rather than generated: `tonic-build` would put
`protoc` in everyone's build for eight small messages, and the field numbers are
pinned against encoded bytes in a test rather than against the declarations.

### What is hooked

A processor can hook any of the seven methods this gateway serves:

| Method | Request phase sees | Response phase sees |
| --- | --- | --- |
| `tools/list` | nothing to rewrite | the merged catalogue |
| `tools/call` | `{"name": "echo", …}` | the `CallToolResult` |
| `prompts/list` | nothing to rewrite | the merged listing |
| `prompts/get` | `{"name": "summarize"}` | the `GetPromptResult` |
| `resources/list` | nothing to rewrite | the merged listing |
| `resources/templates/list` | nothing to rewrite | the merged listing |
| `resources/read` | `{"uri": "memo:insights"}` | the `ReadResourceResult` | A processor keyed only on methods this gateway does
not serve — `logging/setLevel`, `completion/complete` — is reported by `--check`
and at startup, because a guardrail that never fires looks exactly like one that
always passes.

The single-target methods — `tools/call`, `prompts/get`, `resources/read` — run
both phases. The `*/list` methods fan out, so their request phase runs once for
the whole client call and carries no params: a processor can refuse there but
has nothing to rewrite, and filtering a catalogue is response-phase work.

Guardrails run **after** `mcpAuthorization`, so a processor is only consulted
about calls the route was otherwise going to serve — a guardrail should not be
billed for traffic already refused. An upstream *error* skips the response phase entirely: there is no result
to inspect, and asking a guardrail to approve a failure is not a question it can
answer.

### The two phases do not see the same names

On the **request** side a processor sees the **unmuxed** identifier — what the
upstream will actually receive. `alpha_echo` arrives as `{"name": "echo"}` with
`service_names: ["alpha"]`, and `alpha+memo:insights` arrives as
`{"uri": "memo:insights"}`.

On the **response** side it sees what the *client* will get, which for
resources means the federated form: a `resources/read` result reaches the
response phase with its contents already re-qualified to
`alpha+memo:insights`, and a `resources/list` carries federated URIs
throughout.

The asymmetry is not an oversight — each phase shows the form that is
actionable at that point — but a filter written against one form will silently
not match the other, so it is worth knowing which side you are on. Tools never
made this visible because a `CallToolResult` carries no names.

A request-phase rewrite that hands back the federated URI it saw on a listing
is unwrapped rather than passed through to fail, since the upstream only knows
its own URIs.

Note also that a rewrite is **not re-authorized**: a processor that rewrites
`{"name": "summarize"}` to `{"name": "leak"}` gets `leak`, even where a rule or
an `allowTools` entry would have refused it. That is upstream's contract —
"the gateway does not re-run other RBAC on the mutated request" — and it means
a processor with rewrite authority is as trusted as the policy itself.

### A chain is a pipeline, not a vote

Processors run in configuration order, and each one sees what the previous one
produced. A redactor followed by a validator should see redacted input. The
first refusal ends the chain — the processors after it are not consulted, and
neither is the upstream.

### Method matching

`methods` is an allow-list from pattern to phase (`off`, `request`, `response`,
`full`); a method matching no key bypasses the processor. When several patterns
match, the most specific wins: an exact name, then a prefix wildcard
(`tools/*`), then a suffix wildcard (`*/list`), then `*`. Within one kind the
longer pattern wins, and remaining ties break alphabetically so resolution never
depends on map ordering. A pattern that can never match — `a*b`, `**` — is
reported rather than accepted.

### Failing closed

A processor that cannot be reached, exceeds its budget (10s by default), or
answers something unparseable **refuses the call**. `failureMode: failOpen`
reverses that but has to be asked for, for the same reason as `extAuthz`: a
policy service that is down must not silently become an open door.

A refusal carries the processor's own reason and code — `permissionDenied`
becomes JSON-RPC `-32001`, `resourceExhausted` `-32003`, `invalid` `-32600`, and
anything else `-32603`. (`-32002` is skipped: `rmcp` already assigns it to
`RESOURCE_NOT_FOUND`.)

### Header and metadata context

`requestHeaders` bounds what the processor is shown. Note the default: an empty
`allowed` forwards **every** header, which is upstream's reading and the
opposite of `extAuthz.includeHeaders`. `disallowed` always wins, and both match
case-insensitively.

`metadata` is a map of CEL expressions evaluated per call and sent as
`metadata_context`. The context is `jwt` — the verified token's claims — and
`request`, carrying `method` and `headers`. An expression that cannot be
evaluated is dropped rather than failing the call: metadata is context for the
processor, not a decision, and a missing claim should not take a guardrail
offline. One that does not *compile* is a startup failure.

### `headerMutation`

A processor's request-phase answer can also change the headers of the upstream
HTTP request that carries the call — which is how a guardrail passes a resolved
identity to the MCP server behind it:

```
McpRequestResult.header_mutation = { set: [{key: "x-user-id", value: "u-42"}] }
```

It is a **per-call** change, not a connection one. The connection to a target is
dialled once at startup and shared, so the change rides in the request's
extensions, which `rmcp` carries in memory from the peer down to the transport.
The handshake, which happened before any processor was consulted, never carries
it.

Only `mcp:` targets have headers to change. A `stdio` target speaks over a pipe,
so a mutation aimed at one is logged and dropped rather than failing the call.
For `tools/list`, which fans out, the change applies to every target's request —
there is one client call and several upstream ones, and singling one out would
be arbitrary.

Changes accumulate across the chain: a later processor setting a name an earlier
one set wins, a later `remove` cancels an earlier `set`, and vice versa. Within
one processor's answer, repeated `set` entries for one name are joined with
`", "` — the protocol says they form a list replacing the header, and a single
comma-separated field line is how HTTP spells that. A name or value HTTP cannot
represent is skipped with a warning rather than failing the call. A refusal
carries no header changes at all: a request that never happens should leave no
trace on the one that replaces it.

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

## Agent-to-agent

```yaml
policies:
  a2a:
    denyMethods: ["^tasks/cancel$"]
    agentCard:
      url: "https://gateway.example.com/a2a"   # what clients should call
backends:
  - host: "agent-a:9000"
  - host: "agent-b:9000"
```

An `a2a` policy marks a route as carrying [Agent2Agent] traffic. The gateway
does not become an agent — it fronts them — so it takes only
[`rusty_a2a`][rusty_a2a]'s protocol types.

### Method gating

An A2A call names its operation in the JSON-RPC `method` field, so a route can
permit `message/send` and refuse `tasks/cancel`. Denies win, and an empty allow
list means "everything not denied" — the same reading as the MCP tool gate.

A refusal is a **JSON-RPC error object with HTTP 200**, carrying the spec's
`PermissionDenied` code (`-32011`) and echoing the caller's request id. That is
where a JSON-RPC client looks; an HTTP error status would surface as a
transport failure rather than the reason.

A body that is *not* JSON-RPC passes through untouched. A2A also has REST and
gRPC bindings, and refusing everything this gate cannot read would break them
for no security benefit — the gate refuses named methods, it is not a schema
validator.

### Agent card discovery

With `agentCard` set, the gateway serves a merged card at
`/.well-known/agent-card.json`. **The URL rewrite is the point**: an agent
behind a gateway advertises its own address, and a client that reads the card
verbatim goes straight around the gateway, past its authentication, rate limits
and audit trail.

Skills across agents are unioned. Capabilities are **intersected** rather than
unioned — advertising `streaming` because one agent supports it sends a
streaming client to an agent that cannot, and the failure lands on the client.
A skill id offered by two agents is listed once and reported at startup rather
than renamed: unlike an MCP tool name, a skill id is descriptive rather than
what a caller invokes, so qualifying it would misrepresent the agent.

Cards are parsed **leniently**. `rusty_a2a`'s types are transliterated
field-for-field from the normative proto with required fields enforced —
correct for an agent, awkward for a gateway aggregating cards from agents it
does not control. One non-conformant agent is excluded and reported rather than
breaking discovery for the rest. If *no* card can be assembled the well-known
path answers `503`, because serving a half-built card would be worse than
admitting there is none.

Without `agentCard`, the gateway has no opinion about discovery and the
well-known path is proxied like any other request.

**What a merged card does not promise:** the union of skills describes what is
reachable behind the route, not a routing table. This gateway load-balances
across backends by weight and does not route by skill, so a client picking a
skill is not guaranteed to reach the agent offering it. With one backend — a
gateway fronting a single agent — the question does not arise, and the card is
that agent's with its URL corrected.

[Agent2Agent]: https://a2a-protocol.org

## The LLM gateway

```yaml
- name: llm
  matches:
    - path:
        pathPrefix: /v1
  policies:
    backendAuth:
      key: sk-ant-...          # the provider credential, not the caller's
  backends:
    - ai:
        provider:
          anthropic:
            model: claude-sonnet-4   # overrides whatever the caller asked for
```

A client POSTs an ordinary OpenAI chat-completions request and gets an OpenAI
response back, whichever provider actually served it. Switching provider is a
configuration edit rather than a client change — which is the entire point.

### Only Anthropic is translated

For an OpenAI-compatible provider the body is forwarded essentially unchanged:
only `model` is overridden and the credential swapped. That is deliberate. A
typed round-trip would silently drop every field this gateway has not heard of
— tool definitions, `response_format`, `logprobs`, whatever ships next — and a
gateway that quietly deletes half a request is worse than one that refuses it.
`hostOverride` points the same path at any OpenAI-compatible endpoint,
self-hosted or otherwise.

Anthropic gets a real translation, because three differences bite:

- **`max_tokens` is optional for OpenAI and required by Anthropic**, so a valid
  request would be rejected. Translation supplies a default (4096) rather than
  passing the absence through.
- **The system prompt is a message for OpenAI and a top-level field for
  Anthropic.** Left in the message list it is either rejected or, worse,
  silently treated as a user turn. Several system messages are joined rather
  than dropped.
- **Finish reasons use different vocabularies** — `end_turn` is what OpenAI
  calls `stop`. A client switching providers should not have to learn both.

### Streaming

Streams are re-framed, not buffered: an LLM response is the one thing a client
most wants incrementally, and collecting it would turn a token-by-token answer
into a long silence followed by a wall of text.

OpenAI frames pass through byte-identically. Anthropic's event stream is
translated by a small state machine, because the two are not a per-event
mapping: Anthropic sends the id, model and *prompt* tokens once in
`message_start` and the *completion* tokens in `message_delta`, while OpenAI
repeats id and model on every chunk, announces the assistant role exactly once,
and terminates with a literal `data: [DONE]` that is not a chunk at all.

Token usage is reported as a structured log line rather than a metric —
per-request counts keyed by model are unbounded label cardinality, and a client
inventing model names should not be able to take a metrics backend down.

### Errors are passed through

A provider's status and body are returned as they arrived. "invalid api key" is
the useful part, and a gateway that rewrites it as "bad gateway" costs an
afternoon. Errors the gateway itself generates use OpenAI's error envelope, so
a client's existing handling works.

`backendAuth: passthrough` is ignored here: a provider API key is not the
caller's bearer token, and forwarding one as the other would send a user's
credential to OpenAI.

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

## On `rusty_a2a`

A2A support is [`rusty_a2a`][rusty_a2a] — a complete implementation of the
protocol: all 11 operations across JSON-RPC, gRPC and HTTP+JSON/REST, agent
card discovery, JWS signing and push notifications.

The gateway uses about a tenth of it, and that is the right amount. The crate's
centre of gravity is *being* an agent — `AgentExecutor`, task store, lifecycle,
streaming — while a gateway fronts agents. So this takes the protocol types and
leaves the harness alone.

That also settles what looked like the main integration cost. `rusty_a2a` pins
`axum` 0.7, `tonic` and `reqwest` 0.12 against this workspace's 0.8 and 0.13,
which would mean two copies of `reqwest` with separate TLS stacks. All of it is
behind the `client`, `server`, `grpc` and `signing` features, so
`default-features = false` pulls **45 crates with no `axum`, no `tonic` and a
single `reqwest` 0.13**. `build.rs` is gated on `grpc`, so no `protoc` either.

The one real friction is strictness, described under
[Agent-to-agent](#agent-to-agent): the types enforce required fields exactly as
the proto specifies, which is right for an agent and wrong for a gateway
aggregating third-party cards. Hence the lenient parse here.

## Tests

```bash
cargo test --workspace
```

226 tests. The config tests are anchored on YAML taken verbatim from
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

The A2A tests run mock agents that serve real agent cards and record the calls
they received, so "was the method refused" is told apart from "did the agent
see it anyway". The LLM tests stand up a mock provider that records the body it
actually received, so the assertions are about whether the provider got the shape its
API requires rather than merely whether the gateway answered.

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
[rusty_a2a]: https://github.com/baileyrd/rusty_a2a
