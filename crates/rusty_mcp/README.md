# rusty_mcp

A reusable Rust scaffold for building [Model Context Protocol][mcp] servers
against the **2026-07-28** specification.

It owns the parts every MCP server needs and nobody wants to write twice —
argument parsing, transport selection, stderr logging, graceful shutdown — so a
new server is a handler plus a three-line `main`. Built on [`rmcp`] 3.x.

```rust
#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    rusty_mcp::run(|| Ok(MyServer::new())).await
}
```

That binary speaks stdio by default and Streamable HTTP with `--transport http`.

## Why 2026-07-28 matters here

That revision made MCP **stateless**. The `initialize` handshake is gone, along
with `Mcp-Session-Id`, the standalone GET stream, and `Last-Event-ID` stream
resumption. Every request instead carries its own protocol version and client
capabilities in `_meta`.

The practical payoff: an HTTP server built on this scaffold runs behind a plain
round-robin load balancer with **no session affinity and no shared session
store**. Scale it by adding instances.

Two sharp edges this scaffold handles for you:

- **`rmcp`'s `ProtocolVersion::LATEST` still points at `2025-11-25`.** A server
  using `ServerInfo::new` alone advertises the *older* revision even though it
  can speak 2026-07-28. [`rusty_mcp::server_info`] pins the new one. Clients on
  the older revision still negotiate down — there is a test for that.
- **`NeverSessionManager` is the default**, not `LocalSessionManager`, so the
  server refuses to mint sessions at all unless you opt into
  `--legacy-sessions` for pre-2026-07-28 clients.

## Layout

| Path | What it is |
| --- | --- |
| `crates/rusty-mcp` | The reusable scaffold. Depend on this. |
| `crates/rusty-mcp-demo` | An example server exercising every feature. Read this. |
| `template` | A minimal starting point (see [Starting a server](#starting-a-server)) |

Inside the scaffold:

| Module | Responsibility |
| --- | --- |
| `cli` | `--transport`, `--bind`, `--path`, allow-lists, log filter (with env fallbacks) |
| `config` | `ServerConfig` / `HttpConfig`, if you build config yourself |
| `runtime` | `serve()` — wires a handler to stdio or Streamable HTTP |
| `telemetry` | Logging to **stderr**, never stdout |
| `shutdown` | SIGINT / SIGTERM handling |
| `error` | `ServeError`, plus `ToolError` for tool bodies |
| `auth` | OAuth 2.1 resource-server layer (see [Authorization](#authorization)) |
| `resources` | Resource registry with URI templates (see [Resources and prompts](#resources-and-prompts)) |
| `trace` | W3C trace context over `_meta` (see [Tracing](#tracing)) |
| `subscriptions` | Change notifications over `subscriptions/listen` (see [Change notifications](#change-notifications)) |
| `mrtr` | Asking the client for input mid-call (see [Asking the user](#asking-the-user)) |
| `pagination` | The cursor every paginated list shares (see [Lists are paginated](#lists-are-paginated)) |
| `routers` | Paginating replacements for `#[tool_handler]` / `#[prompt_handler]` |
| `completion` | Argument completion (see [Completion](#completion)) |
| `otel` | OTLP span and metric export (see [Exporting spans](#exporting-spans)) |
| `limits` | Concurrency and timeout shedding (see [Limits](#limits)) |
| `tasks` | Tasks extension for long-running tools (see [Long-running tools](#long-running-tools)) |

## Starting a server

```bash
cargo generate --git https://github.com/baileyrd/rusty_mcp template
```

You get a compiling MCP server with two example tools, a test that drives it
through a real client, and a README covering the rest. No `cargo generate`?
Copy `template/`, rename `{{ 'Cargo' }}.toml` to `Cargo.toml`, and replace
`{{project-name}}` and `{{description}}`.

That filename is deliberate. Cargo scans a git checkout for manifests when
resolving a git dependency, and it does so independently of the workspace
`exclude` list — so a real `Cargo.toml` holding `name = "{{project-name}}"`
printed `error: invalid character` on every build of every project depending on
this crate. Harmless to the build, pure noise in someone else's terminal.
`cargo generate` renders Liquid in filenames as well as contents, so
`{{ 'Cargo' }}.toml` arrives as `Cargo.toml` while staying invisible to cargo
here.

The demo crate is the other half of this: `template` is where you start,
`crates/rusty-mcp-demo` is what to read when you need resources, prompts,
tasks, subscriptions or MRTR wired up for real.

### The template cannot rot

A template is a second copy of the API surface, and second copies go stale
silently — this repo has already shipped one instance. The README told readers:

<!-- check-versions: ignore -->
```toml
rusty-mcp = { git = "...", tag = "v0.2.0", features = ["otel"] }
```

That tag predated the `otel` feature, so following the instructions verbatim
did not compile.

So CI generates the project and builds, lints and tests it on every pull
request, against the working tree rather than the published tag. A library
change that breaks the template fails the PR that made it.

It has a blind spot by design: because it rewrites the dependency to a **path**
so a library change fails the PR that made it, the **git** dependency every
real consumer uses is never resolved. That is exactly where the
`{{project-name}}` manifest noise above hid — found only once v0.4.0 existed to
depend on.

`check-published-tag.sh` closes it, and runs automatically on every tag push:

```bash
./scripts/check-published-tag.sh v0.5.0
```

It runs the documented `cargo generate --git … --tag <tag> template`, leaves
the git dependency exactly as the template ships it, and builds and tests the
result.

The important part is what it counts as failure. **The v0.4.0 build succeeded**
— it just printed an error while doing so, which is invisible to an exit code.
So the check scans what cargo actually printed and fails on an error line even
when the command returned zero. That behaviour is proven twice over: a
`--self-test` on synthetic output, and the fact that pointing it at `v0.4.0`
still reproduces the original failure while `v0.5.0` passes.

`check-versions.sh` separately asserts that every `tag = "vX.Y.Z"` in the docs
and the template names the current crate version — which is exactly the failure
above, caught this time.

Prose that *quotes* an old snippet, like the one above, is exempted explicitly:

```markdown
<!-- check-versions: ignore -->
```

That covers the rest of its line, the line after, and — when that next line
opens a fenced block — the whole block. It exists because the check fired three
times on passages explaining the very bug it prevents, and each time the fix
was to make the documentation vaguer. A check that degrades the docs every time
it fires is taxing the wrong thing.

Nothing is inferred beyond those rules, and every exemption is listable:

```bash
./scripts/check-versions.sh                    # check
./scripts/check-versions.sh --list-exemptions  # what is exempted, and where
./scripts/check-versions.sh --self-test        # prove both directions work
./scripts/check-template.sh
```

The self-test runs in CI. A check nobody has watched fail is a check nobody
knows works — and this one had never been seen to reject anything.

## Writing tools

Group tools by topic, one module each. Every module contributes a router; the
server merges them with `+`, so adding a topic never means editing a central
registry.

```rust
#[tool_router(router = calculator_tools, vis = "pub(crate)")]
impl DemoServer {
    #[tool(description = "Divide two integers, returning quotient and remainder.")]
    pub async fn divide(
        &self,
        Parameters(BinaryOp { a, b }): Parameters<BinaryOp>,
    ) -> Result<Json<DivideResult>, ErrorData> {
        if b == 0 {
            return Err(ToolError::invalid("cannot divide by zero").into());
        }
        Ok(Json(DivideResult { quotient: a / b, remainder: a % b }))
    }
}
```

```rust
// crates/rusty-mcp-demo/src/server.rs
tool_router: Self::calculator_tools() + Self::text_tools(),
```

Notes that save time:

- **Return `Result<T, ErrorData>`**, not your own error type — `rmcp` requires
  it. `ToolError` is a shorthand that converts through `?` and `.into()`.
- **Doc comments on argument structs become the JSON Schema descriptions** the
  model reads. They are part of the interface, not decoration.
- **Prefer `Json<T>` for anything structured.** It fills `structuredContent`
  alongside the text block, so clients can consume data without reparsing.
- **A protocol error is not a tool failure.** `ErrorData` means the call could
  not be processed. A tool that ran but failed in its domain ("no such user")
  should return a normal result with `isError: true`, so the model can react.

## Resources and prompts

Tools are one of three things a server exposes. The other two are **resources**
(data the client can read) and **prompts** (templates a *user* invokes, usually
as a slash command — the model never picks one on its own).

`rmcp` gives prompts the same router treatment as tools, so they compose the
same way:

```rust
#[prompt_router(router = "demo_prompts", vis = "pub(crate)")]
impl DemoServer {
    #[prompt(name = "summarize", description = "Summarize text.")]
    pub async fn summarize(&self, Parameters(args): Parameters<SummarizeArgs>)
        -> Vec<PromptMessage> { /* ... */ }
}
```

Note `prompt_router` takes its router name as a **string literal**, where
`tool_router` takes an identifier — an easy five minutes to lose.

Resources have no router in `rmcp`, so this crate provides one:

```rust
use rusty_mcp::resources::{ReadRequest, ResourceRegistry};

let resources = ResourceRegistry::new()
    // Fixed content.
    .with_text(Resource::new("config://demo", "demo-config"), r#"{"a":1}"#)
    // Generated per read.
    .with_reader(Resource::new("status://uptime", "uptime"), |req: ReadRequest| async move {
        Ok(vec![ResourceContents::text(uptime(), req.uri.clone())])
    })
    // A templated family.
    .with_template(
        ResourceTemplate::new("db://tables/{table}", "table-schema"),
        |req: ReadRequest| async move {
            let table = req.param("table").unwrap_or_default();
            /* ... */
        },
    );
```

Then, alongside the handler attributes:

```rust
#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for DemoServer {
    fn get_info(&self) -> ServerInfo { /* enable_resources(), enable_prompts() */ }
    rusty_mcp::forward_resource_methods!(resources);
}
```

The registry handles what you'd otherwise write by hand: cache hints on all
three responses, concrete URIs matching ahead of templates, and not-found
errors — which `rmcp` renders as `-32602` for 2026-07-28 peers and the legacy
`-32002` for older ones, since the code changed in this revision.

### Template variables never cross `/`

`db://tables/{table}` will not match `db://tables/../../etc/passwd`. That URI
falls through to not-found rather than reaching your reader with a traversal
payload — which matters most for the case people reach for first, a
filesystem-backed resource. Values are percent-decoded after matching, so the
decoding cannot reintroduce a separator.

Templates are RFC 6570 **level 1**: `{var}` placeholders only, no operators.

### Lists are paginated

All four list methods page at `DEFAULT_PAGE_SIZE` (100). Resources take
`.with_page_size(n)`; tools and prompts take an optional second macro argument.

```rust
impl ServerHandler for MyServer {
    fn get_info(&self) -> ServerInfo { /* ... */ }

    rusty_mcp::forward_resource_methods!(resources);
    rusty_mcp::forward_tool_methods!(tool_router);
    rusty_mcp::forward_prompt_methods!(prompt_router);
}
```

**Those last two replace `#[tool_handler]` and `#[prompt_handler]`** — they do
not compose with them. `rmcp`'s generated `list_tools` and `list_prompts` take
a cursor, discard it, and return everything with `next_cursor: None`, and
neither can be overridden in place:

- `#[prompt_handler]` scans for an existing `list_prompts` and **replaces its
  body**, so a hand-written paginating version is silently discarded.
- `#[tool_handler]` guards on `has_method("list_tools")`, which looks like an
  override would work — but attribute macros expand *before* `macro_rules!`
  invocations inside the item, so it only ever sees an unexpanded macro call.
  You get `E0201: duplicate definitions`.

The first failure is silent, which is why neither attribute is worth keeping.
Write `get_info` yourself; `rusty_mcp::server_info` makes it a one-liner.

One cursor implementation serves all four, and a cursor minted by one sequence
is rejected by the others — there is a tag byte for exactly that.

The cursor carries a **key, not an index**, and pages are ordered by key rather
than by registration order. Two things follow, and both are the reason for the
choice:

- A fabricated cursor names some position in the key space instead of an offset
  into a slice, so there is no out-of-range read to defend against.
- An entry added or removed between requests cannot shift the page boundary. A
  cursor stays valid even when the entry it names is the one that was deleted.

A cursor that does not decode — wrong encoding, wrong version, or minted for
the other sequence — is `-32602`, not a silently empty page. A client that has
lost its place should be told.

## Completion

`completion/complete` is what lets a client suggest values for a prompt
argument or a resource-template variable, instead of the user having to already
know that `db://tables/{table}` wants `users`. `rmcp` ships the wire types but
no router, so this crate provides one:

```rust
use rmcp::model::Reference;
use rusty_mcp::completion::{CompletionRegistry, CompletionRequest};

let completions = CompletionRegistry::new()
    // A fixed list.
    .with_values(
        Reference::for_prompt("explain-error"),
        "language",
        ["rust", "python", "typescript"],
    )
    // Computed per request — and able to depend on the arguments already
    // filled in, which is what makes a `column` argument narrow to the
    // `table` the user just picked.
    .with_completer(
        Reference::for_resource("db://{schema}/{table}"),
        "table",
        |req: CompletionRequest| async move {
            Ok(tables_in(req.argument("schema").unwrap_or("public")))
        },
    );
```

```rust
impl ServerHandler for DemoServer {
    fn get_info(&self) -> ServerInfo { /* ... enable_completions() */ }
    rusty_mcp::forward_completion_methods!(completions);
}
```

A completer returns **candidates**, not a finished result. Prefix matching
(case-insensitive), sorting, the spec's 100-value cap and the `hasMore` flag
are applied for you, and `total` reports the count *before* the cap so a client
can tell "these are the only three" from "here are 100 of many".

### Advertise it or nothing happens

`enable_completions()` is not optional decoration. A client that is not told
the server completes will never call the method, and the registry is dead
weight.

### A typo is silent without `dangling()`

Registering a completion against a prompt name that does not exist is accepted
without complaint, and the client — asking under the real name — gets an empty
list, which is indistinguishable from having nothing to suggest. Nothing ever
errors.

Checking at registration time would mean the completion registry holding a
reference to the prompt router and the resource registry, which inverts the
composition everything else here uses. So the check is a call instead:

```rust
let dangling = completions.dangling(&prompt_names, &resources.template_uris());
assert!(dangling.is_empty(), "completions that will never fire: {dangling:?}");
```

The demo runs this both as a test — so a prompt rename that orphans a
completion fails CI — and at startup, where it logs a warning.

### It runs while someone is typing

Clients call completion per keystroke, or close to it. A completer that queries
a database on every call is felt directly as input lag; cache the candidate
list and let the registry do the filtering.

## Shared state

Streamable HTTP builds a **fresh handler per request**, so anything that must
outlive a single call goes behind an `Arc` captured by the factory closure:

```rust
let state = Arc::new(DemoState::default());
rusty_mcp::serve(move || Ok(DemoServer::with_state(Arc::clone(&state))), config).await
```

## Running it

```bash
# stdio — what Claude Desktop and Claude Code launch locally
cargo run -p rusty-mcp-demo

# Streamable HTTP
cargo run -p rusty-mcp-demo -- --transport http --bind 127.0.0.1:8080
```

Every flag has an environment fallback (`MCP_TRANSPORT`, `MCP_BIND`,
`MCP_PATH`, `MCP_ALLOWED_HOSTS`, …). `RUST_LOG` overrides `--log`.

### Talking to it directly

A 2026-07-28 request carries `_meta` *and* the matching `MCP-Protocol-Version`
header — the transport rejects a mismatch with `-32020`:

```bash
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' \
  -H 'Mcp-Method: tools/list' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}}}}'
```

### Wiring into a client

```json
{
  "mcpServers": {
    "rusty-mcp-demo": {
      "command": "/path/to/target/release/rusty-mcp-demo"
    }
  }
}
```

## Deploying over HTTP

The `Host` allow-list defaults to **loopback only**, which guards local servers
against DNS rebinding. A public deployment must set its own hostnames, and any
browser-reachable server should set origins too:

```bash
rusty-mcp-demo --transport http --bind 0.0.0.0:8080 \
  --allowed-host mcp.example.com \
  --allowed-origin https://app.example.com
```

### Limits

The request body cap is on by default — 4 MiB, `--max-body-bytes`. It is
enforced incrementally per body frame, so an oversized request gets `413`
without ever being fully read into memory.

Concurrency and timeouts are **off** by default and worth turning on for
anything public:

```bash
rusty-mcp-demo --transport http --bind 0.0.0.0:8080 \
  --max-concurrent 256 \
  --request-timeout-secs 30
```

or in code, via `HttpConfig::limits`:

```rust
use rusty_mcp::limits::LimitsLayer;

http.limits = Some(
    LimitsLayer::new()
        .with_max_concurrent(256)
        .with_timeout(Duration::from_secs(30)),
);
```

Unlike the `Host` allow-list these have no safe default: the right number
depends entirely on what your tools do, and shipping one would be a silent
regression for any server already handling more.

#### Shedding, not queueing

Over the limit, a request gets `503` with `Retry-After` immediately. It is not
queued. A queue in front of an overloaded server converts a capacity problem
into a latency problem — every client waits longer, times out, and retries,
which is how a brief spike becomes a sustained outage. Refusing quickly lets a
client back off while the requests already accepted still finish on time.

This is also why the limit is not applied in `poll_ready`: returning `Pending`
there is backpressure, and backpressure means the caller waits.

#### Layer order is outside-in

**limits → metrics → authorization → handler**, cheapest rejection first.

A shed request costs a semaphore try-acquire and nothing else. Inside the auth
layer it would pay for a token validation — and with `JwtValidator`, possibly a
JWKS lookup — before being told there was no capacity for it anyway. There is a
test that fills the server, sends an unauthenticated request, and asserts it
comes back `503` rather than `401`.

The trade is that shed requests are invisible to the metrics layer. That's the
right way round: the `503` rate is something your load balancer already sees,
whereas a rejected token is only visible here.

#### What the timeout does not cover

Time to *produce* a response, not time to stream one. A long-lived
`subscriptions/listen` is unaffected — the transport returns the SSE response
promptly and streams events afterwards, so the part being timed is short even
though the request lives for hours. This is verified rather than assumed: a
test holds a subscription open for five times the timeout and then requires a
published change to arrive.

A tool running **inline** does hold the response open for its whole duration
and will be cut off with `504`. That is the intended behaviour, and the reason
to reach for [tasks](#long-running-tools) for anything slow — a task releases
its permit as soon as the handle goes back to the client, so long work never
counts against the concurrency limit.

## Long-running tools

A tool that takes minutes should not hold a request open for minutes. The
[tasks extension][tasks] (`io.modelcontextprotocol/tasks`, SEP-2663) lets the
server answer `tools/call` with a **task handle**; the client polls `tasks/get`
until it settles. In 2026-07-28 this moved out of the core protocol into an
opt-in extension, and the blocking `tasks/result` became polling.

Add a `TaskSupport` to your server, advertise the capability, and forward the
three task methods:

```rust
use rusty_mcp::tasks::{TaskPolicy, TaskSupport};

#[derive(Clone)]
pub struct MyServer {
    tasks: TaskSupport,
    tool_router: ToolRouter<Self>,
}

// Only name the tools that are actually slow.
let tasks = TaskSupport::with_policy(TaskPolicy::named(["countdown"]));

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MyServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info("my-server", "0.1.0",
            ServerCapabilities::builder().enable_tools().enable_tasks().build())
    }

    rusty_mcp::forward_task_methods!(tasks);
}
```

Then let `TaskSupport::run` decide per call, inside an ordinary `#[tool]` fn:

```rust
#[tool(description = "Count down in steps, slowly.")]
pub async fn countdown(
    &self,
    Parameters(args): Parameters<CountdownArgs>,
    ctx: RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    self.tasks.run(&ctx, COUNTDOWN, move |task| countdown_body(args, task)).await
}
```

Two things make that work: taking `RequestContext` gives the tool the
per-request client capabilities, and returning `CallToolResponse` is what lets
it hand back a handle instead of a result.

### Why one body, not two

**A task handle may only go to a client that declared the extension.** Send one
to a client that did not and dispatch rejects it with `-32021`, which surfaces
as a confusing failure rather than graceful degradation. So every task-capable
tool has to work both ways.

`run` picks the path and executes the same body either way. Write it against
`TaskCtx`, which degrades to no-ops off-task — `set_status_message` does
nothing, and `cancelled()` never resolves, so it stays safe as a `select!` arm:

```rust
tokio::select! {
    _ = ctx.cancelled() => return Err(TaskExit::Cancelled),
    _ = tokio::time::sleep(step) => {}
}
```

Writing the two paths separately is how they drift — a fix lands on one and not
the other.

### Notes

- **Clone one `TaskSupport`, don't construct per request.** Streamable HTTP
  builds a fresh handler per request while tasks outlive the call that created
  them; a new manager per request means every poll misses.
- **Cancellation is cooperative.** `tasks/cancel` is acknowledged immediately
  and the task settles as `cancelled` only if its body checks. A body that
  ignores `cancelled()` runs to completion regardless.
- **TTL bounds memory**, defaulting to an hour. Too short and a slow-polling
  client loses the result it was waiting for.
- **Drain tasks on shutdown.** Without a hook the process exits when the
  transport closes and in-flight tasks are dropped mid-step:

  ```rust
  let config = config.with_shutdown_hook({
      let tasks = tasks.clone();
      move || {
          let tasks = tasks.clone();
          Box::pin(async move {
              let abandoned = tasks.drain(Duration::from_secs(10)).await;
              if abandoned > 0 {
                  tracing::warn!(abandoned, "aborted tasks still running at shutdown");
              }
          })
      }
  });
  ```

  `drain` returns how many were still running when the grace period expired —
  a non-zero count means clients polling those ids will never see a result.

[tasks]: https://modelcontextprotocol.io/seps/2663-tasks-extension

## Authorization

Set `HttpConfig::auth` and the endpoint becomes an OAuth 2.1 **resource
server**: `RequireAuthLayer` guards the MCP route, and the RFC 9728 Protected
Resource Metadata document is published *unauthenticated* alongside it.

```rust
use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};

let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(my_validator))?
    .with_authorization_servers(["https://auth.example.com"])
    .with_scopes_supported(["mcp:read"])
    .with_required_scopes(["mcp:read"]);

let config = ServerConfig {
    transport: Transport::Http(HttpConfig { auth: Some(Arc::new(auth)), ..Default::default() }),
    ..Default::default()
};
```

### Validating tokens

The `jwt` feature ships `JwtValidator`, which is what most deployments want —
point it at your authorization server and it verifies signature, expiry,
not-before and issuer against a cached JWKS:

```toml
rusty-mcp = { git = "...", features = ["jwt"] }
```

```rust
let validator = JwtValidator::builder(
    "https://auth.example.com",
    "https://auth.example.com/.well-known/jwks.json",
).build()?;
```

Defaults worth knowing: RS256 and ES256 only (symmetric algorithms have no
place with a JWKS), 60s clock-skew leeway, a 5-minute JWKS cache, and a 30s
floor between refetches provoked by an unknown `kid` — without that floor,
random `kid` values would let anyone drive unbounded outbound requests to your
authorization server.

`JwtValidator` reads `aud` into the token and lets the **layer** compare it
against the configured resource, rather than checking it itself. Two places
checking the same value means two things to keep in sync, and the one that
quietly stopped matching would be the one nobody noticed.

For anything else — RFC 7662 introspection, a lookup in your own store —
implement `TokenValidator`. `StaticTokenValidator` is for tests and local
development only; it does no cryptography.

### What the layer enforces

| Situation | Status | `WWW-Authenticate` |
| --- | --- | --- |
| No `Authorization` header | 401 | `Bearer scope=…, resource_metadata=…` (no `error`, per RFC 6750 §3.1) |
| Not `Bearer`, or empty token | 400 | `error="invalid_request"` |
| Token fails validation or is expired | 401 | `error="invalid_token"` |
| Token minted for another resource | 401 | `error="invalid_token"` |
| Valid token, missing scopes | 403 | `error="insufficient_scope", scope="…"` |
| Validator itself is down | 503 | *(none)* |

Three of those are worth calling out:

- **Audience binding is enforced by the layer, not left to your validator.**
  The spec's "MCP servers **MUST NOT** accept or transit any other tokens" is
  aimed at a confused deputy — someone replaying a token minted for a different
  service to borrow this server's privileges. A validator that forgets the
  `aud` check would reopen that hole, so `AuthConfig::resource` is checked
  centrally. If your validator provably binds the audience itself, say so with
  `VerifiedToken::audience_checked_by_validator()`.
- **Insufficient scope is 403, not 401.** Re-authenticating doesn't help; the
  client needs a *broader* token. The challenge names every missing scope at
  once, because drip-feeding them forces one authorization round trip per scope.
- **A validator outage is 503 with no challenge.** A 401 would tell the client
  its perfectly good token is bad and send the user through a pointless login.

### The metadata URL is not where you'd guess

RFC 9728 §3.1 inserts the well-known segment *before* the resource path. For
resource `https://mcp.example.com/mcp` the document lives at:

```
https://mcp.example.com/.well-known/oauth-protected-resource/mcp
```

`AuthConfig::metadata_url()` and `metadata_path()` derive this for you.

### Per-tool scopes

`required_scopes` guards the whole endpoint. For finer grain, leave it empty and
check inside a tool — the layer puts the `VerifiedToken` in the request
extensions, which the transport forwards as `http::request::Parts`:

```rust
let token = ctx
    .extensions
    .get::<http::request::Parts>()
    .and_then(|parts| parts.extensions.get::<VerifiedToken>());
```

Scope checks are plain set containment. If your scheme has hierarchies where a
broader scope implies narrower ones, expand the implied scopes in your validator
so they land in `VerifiedToken::scopes`.

### Not included

Only the resource-server half is here. Running an authorization server, the
client-side flow (PKCE, `resource` parameter, RFC 9207 `iss` validation), and
token issuance are all separate concerns — front this with a real
authorization server such as Keycloak, Auth0, or WorkOS.

Authorization is HTTP-only by design: the spec says stdio servers **SHOULD NOT**
use it and should read credentials from the environment instead.

## Asking the user

When a tool needs something from the client mid-call — a confirmation, a choice
— 2026-07-28 does not let the server send it a request. The server *returns* an
input request, the client answers, and **the client retries the original call**
carrying its answers plus the `requestState` the server handed back
([SEP-2322][mrtr]).

The protocol is stateless, so that retry lands on a handler which remembers
nothing. Everything needed to resume has to survive a round trip **through the
client**.

```rust
use rusty_mcp::mrtr::{InputGate, Turn};

#[tool(name = "drop_table", description = "Drop a table, after confirming.")]
pub async fn drop_table(
    &self,
    Parameters(DropTableArgs { table }): Parameters<DropTableArgs>,
    RequestState(request_state): RequestState,
    InputResponses(responses): InputResponses,
) -> Result<CallToolResponse, ErrorData> {
    match self.confirmations.turn(DROP_TABLE, request_state.as_deref(), responses.as_ref())? {
        Turn::Fresh => Ok(CallToolResponse::InputRequired(self.confirmations.ask(
            DROP_TABLE,
            &PendingDrop { table },
            InputGate::<PendingDrop>::confirm("confirm-drop", "Really drop it?"),
        )?)),

        Turn::Resumed { state, answers } => {
            if answers.accepted("confirm-drop") { /* act on state.table */ }
            # todo!()
        }
    }
}
```

### `requestState` is untrusted

The client echoes it back verbatim, so a caller can change it. A server that
keeps meaningful data there — a table name, a user id, an amount — and reads it
back unverified has handed the client a way to rewrite the server's own memory
mid-operation.

`InputGate` seals it with HMAC-SHA256 and additionally:

- **binds the state to the tool that created it**, so an answer given while
  confirming one operation cannot authorize a different one;
- **expires it** (5 minutes by default), so an answer cannot be replayed later;
- **counts rounds**, so a server/client loop terminates.

Two habits that matter more than the sealing:

- **Act on the sealed state, not the retry's arguments.** A client can change
  them between rounds; the user confirmed what was in the *prompt*. There's a
  test (`the_confirmed_table_comes_from_the_sealed_state`) pinning this.
- **Only an explicit yes is consent.** `accepted()` returns false for a
  decline, a cancel, a missing answer, or a malformed one — a dropped
  connection must not read as approval.

### The signing key must be shared

Use a stable per-deployment secret from configuration, not a per-process random
value. With several instances behind a load balancer, the retry will not land on
the instance that issued the state.

### Scope

MRTR carries sampling, elicitation and roots. **Sampling and roots are
deprecated** in this revision, so `InputGate` gives elicitation the typed
helpers and leaves the other two reachable through the raw `InputRequests` map
rather than encouraging them.

[mrtr]: https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr

## Change notifications

2026-07-28 replaced the standalone HTTP GET stream and
`resources/subscribe`/`resources/unsubscribe` with a single long-lived request:
the client POSTs `subscriptions/listen`, opts in to the categories it wants, and
the server streams notifications on that request's response until it ends.

`rmcp` handles the protocol. What it leaves you is the loop — waiting for
something in your application to change and forwarding it to every client
currently listening. `ChangeBroadcaster` is that loop:

```rust
use rusty_mcp::subscriptions::ChangeBroadcaster;

// Anywhere in your application:
changes.resources_changed();
changes.resource_updated("config://demo");
```

```rust
impl ServerHandler for MyServer {
    fn get_info(&self) -> ServerInfo { /* ... */ }
    rusty_mcp::forward_subscription_methods!(changes);
}
```

Publishing is infallible and non-blocking — having no listeners is the normal
state, and application code shouldn't have to care whether anyone is subscribed.

### Advertise what you intend to send

`rmcp` intersects the client's requested filter with the capabilities from
`get_info`, so **a category you forget to advertise is dropped without error**:
the subscription succeeds and simply stays quiet. If notifications aren't
arriving, check the flags first.

```rust
ServerCapabilities::builder()
    .enable_resources()
    .enable_resources_list_changed()   // required for resourcesListChanged
    .enable_resources_subscribe()      // required for per-URI updates
    .enable_prompts_list_changed()
    .enable_tool_list_changed()
    .build()
```

### Clone one broadcaster

Same rule as `TaskSupport`: Streamable HTTP builds a fresh handler per request,
so constructing a new broadcaster per handler would leave every subscription
connected to a channel nobody publishes to.

### On lag

A slow listener that overflows its buffer gets `Lagged`. Rather than failing the
subscription, `run` re-announces each accepted list-changed category — these are
"re-fetch" signals, so the client ends up with fresh lists either way. Missed
per-resource updates can't be recovered that way; a client needing exact update
events should keep up or use a larger buffer.

## Tracing

The 2026-07-28 spec reserves three **bare** `_meta` keys — `traceparent`,
`tracestate` and `baggage` — as an explicit exception to the reverse-DNS prefix
rule, so MCP interoperates with existing OpenTelemetry tooling ([SEP-414]).

`rmcp` carries those strings; `rusty_mcp::trace` gives them meaning:

```rust
use rusty_mcp::trace::TraceContext;

let span = TraceContext::from_request(&ctx)
    .map(|tc| tc.span("tools/call"))
    .unwrap_or_else(|| tracing::info_span!("tools/call"));
let _guard = span.enter();
```

Every log line inside that span carries `trace_id` and `parent_span_id`, so a
request can be followed from the client, through this server, to whatever it
calls next. Propagating onward is `tc.child(new_span_id)` then
`child.apply_to(&mut params)`.

On its own this correlates logs. For spans in a trace viewer, enable the `otel`
feature — see below.

### Exporting spans

The `otel` feature adds a real OTLP pipeline and the one thing `tracing` cannot
do alone — adopting a **remote parent**, so this server's spans become children
of the caller's rather than a separate trace:

```toml
rusty-mcp = { git = "...", tag = "v0.5.0", features = ["otel"] }
```

```rust
use rusty_mcp::otel::{self, OtelConfig};

let guard = otel::init(
    OtelConfig::new("my-mcp-server").with_endpoint("http://localhost:4317"),
    "info",
)?;

// In a handler: `span()` records the ids; `attach_parent` makes the edge real.
let span = tc.span("tools/call");
tc.attach_parent(&span);
```

Three things that decide whether this works in practice:

- **Flush before exiting.** Spans are batched, so whatever is buffered is lost
  if the process just exits. This is the most common way to end up staring at
  an empty collector while insisting the code is instrumented. Wire
  `guard.shutdown_hook()` into `ServerConfig::with_shutdown_hook` and it happens
  on SIGTERM.
- **Sampling follows the caller.** The sampler is parent-based: if the caller
  sampled, so do we. Deciding independently is how traces end up half-recorded,
  with gaps exactly where a service made its own choice. `with_sample_ratio`
  only applies to traces that *start* here.
- **A collector being down must not matter.** Telemetry is not load-bearing;
  export failures are logged and dropped, never surfaced to a caller. There's a
  test pointing the exporter at a closed port.

If your process already builds a subscriber, use `otel::pipeline()` for the
tracer and add the layer yourself — `otel::init()` installs a global subscriber
and only the first call in a process wins.

### Metrics

Traces are what you read *after* being paged. Metrics are what page you — and
they are not recoverable from the spans here, because the parent-based sampler
means the recorded spans are the ones your **callers** chose to sample, which
is not a representative sample of this server's traffic.

The same pipeline exports both. Metrics are on by default; `without_metrics()`
turns them off.

```rust
let guard = otel::init(OtelConfig::new("my-mcp-server"), "info")?;
let instruments = guard.instruments().expect("metrics are enabled");

// Request metrics: a tower layer on the HTTP endpoint.
http.metrics = Some(
    McpMetricsLayer::new(Arc::clone(instruments))
        .with_known_names(["add", "divide", "summarize"]),
);

// Task metrics: the tasks extension needs its own, since the work outlives
// the request that handed out the handle.
let tasks = TaskSupport::new().with_metrics(Arc::clone(instruments));
```

You get `mcp.server.requests` (by method, name and outcome),
`mcp.server.request.duration`, `mcp.server.requests.in_flight`, and
`mcp.server.tasks.{started,finished}`.

#### Cardinality is the thing to get wrong

A metrics backend does not degrade gracefully when a label set explodes — it
falls over, and takes the rest of your metrics with it. Every label here comes
from a closed set fixed before any request arrives:

- `mcp.method` is matched against the methods the spec defines; anything else
  is `other`. A client cannot mint a label by inventing a method.
- `mcp.name` is recorded only for `tools/call` and `prompts/get`, and only for
  names passed to `with_known_names`. **Calling a tool that does not exist must
  not create a label** — the call fails, but a naive implementation records the
  label first.
- `resources/read` puts the **URI** in `Mcp-Name`, and task methods put the
  task id there. Neither is ever used as a label. Both are unbounded.

Both guarantees have tests that drive forged names through the layer and assert
they never reach the collector.

#### `outcome` is transport-level

The layer sees HTTP, so a tool returning `isError: true` counts as `ok` —
JSON-RPC puts application errors in a `200` body, and a middleware that does
not parse bodies cannot see them. Count those in the tool if you need them.
`401` and `403` get their own `unauthorized` outcome, because an auth problem
is a different page at 3am than a malformed request.

#### The layer sits outside authorization

Deliberate, and tested end to end: a request rejected with a `401` is still
counted. Mounted inside the guard, a flood of bad tokens would look like no
traffic at all.

Labels come from the SEP-2243 `Mcp-Method` and `Mcp-Name` headers, so no
request body is ever parsed. This covers HTTP only — a stdio server is one
process serving one client over a pipe, with no middleware position to occupy.

### Two behaviours worth knowing

- **A malformed `traceparent` is treated as absent, not as an error.** W3C
  requires starting a fresh trace rather than propagating something
  unparseable. Erroring instead would let a broken upstream fail your requests;
  passing the raw bytes through would corrupt every trace downstream. Parsing
  is strict: all-zero ids, wrong field widths, non-hex and version `ff` are all
  rejected, while unknown future versions still parse.
- **Baggage is untrusted.** It crosses service boundaries unauthenticated, so
  anyone who can reach the client can put values in it. Use it for diagnostics,
  never for authorization. `Baggage` enforces the W3C caps (180 entries, 8 KiB)
  so an oversized header cannot exhaust memory.

[SEP-414]: https://modelcontextprotocol.io/specification/2026-07-28/basic/index#opentelemetry-trace-context

## Development

```bash
cargo test --workspace     # unit, integration (both transports), and doc tests
cargo clippy --workspace --all-targets
cargo fmt --all
```

The HTTP tests run over a real TCP socket with the `rmcp` client, so they cover
the whole path: `serve` → axum → `StreamableHttpService` → handler.

## Notes on the client side

`rmcp`'s default client still uses the legacy `initialize` handshake. To get a
stateless 2026-07-28 client, ask for it explicitly:

```rust
ClientInfo::default()
    .serve_with_lifecycle(
        transport,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await?
```

## Using it

Neither crate is published to crates.io. Depend on it by git tag:

```toml
rusty-mcp = { git = "https://github.com/baileyrd/rusty_mcp", tag = "v0.5.0" }
```

Add `features = ["jwt"]` for the JWKS-backed token validator.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## License

Apache-2.0. See [LICENSE](LICENSE).

[mcp]: https://modelcontextprotocol.io/specification/2026-07-28
[`rmcp`]: https://crates.io/crates/rmcp
[`rusty_mcp::server_info`]: crates/rusty-mcp/src/lib.rs
