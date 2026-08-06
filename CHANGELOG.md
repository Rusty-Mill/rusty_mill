# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog][kac] and [Semantic Versioning][semver].

Neither crate is published to crates.io — consume `rusty-mcp` by git tag:

```toml
rusty-mcp = { git = "https://github.com/baileyrd/rusty_mcp", tag = "v0.4.1" }
```

Being `0.x`, the API may still break in a minor release. Breaking changes will
be called out here.

Targets MCP specification [2026-07-28][spec], on [`rmcp`][rmcp] 3.x.

## [Unreleased]

### Added

- An exemption marker for `scripts/check-versions.sh` ([#28]):

  ```markdown
  <!-- check-versions: ignore -->
  ```

  It covers the rest of its own line, the line after it, and — when that next
  line opens a fenced code block — the whole block. That last rule is there
  because the most useful thing to write is the snippet that actually misled
  someone, and a marker cannot go inside a fence without rendering as code.

  The check matches textually, so it had fired three times on prose *quoting*
  an old snippet — every time in a passage explaining the bug the check exists
  to prevent — and each time the fix was to reword the documentation into
  something vaguer. All three passages are restored to showing the real
  snippet.

  Nothing is inferred beyond those rules. `--list-exemptions` prints every one
  with its file and line, so exempting something cannot be quiet.

  `--self-test` asserts both directions on throwaway fixtures — unmarked stale
  tags rejected, marked ones accepted, same-line and fenced markers honoured,
  the exemption ending at the closing fence, and exempting *everything* still
  failing rather than silently guarding nothing. It runs in CI. This check had
  never been observed to reject anything before now.

### Fixed

- **`tools/list` and `prompts/list` accepted a pagination cursor and ignored
  it** ([#26]). 0.4.0 fixed this for resources and deferred the rest with a
  note to check whether `rmcp`'s routers paginate. They do not: both generated
  methods take a `PaginatedRequestParams`, discard it, return `list_all()` and
  set `next_cursor: None`. Same consequence as before — a client that pages
  sees a full first page with no cursor and concludes it has everything.

  Leaving it was worse than not having noticed. Between 0.4.0 and now, one
  third of the list surface paged and nothing said which third.

### Added

- `routers::forward_tool_methods!` and `routers::forward_prompt_methods!`,
  paginating on the same cursor as resources.

  **They replace `#[tool_handler]` and `#[prompt_handler]` rather than
  composing with them**, for two separate reasons, both found by trying:

  - `#[prompt_handler]` scans the impl for `list_prompts` and *replaces its
    body* with the generated one, so a hand-written override is silently
    discarded.
  - `#[tool_handler]` guards on `has_method("list_tools")`, which looks like an
    override would work — but attribute macros expand before `macro_rules!`
    invocations inside the item, so it only ever sees an unexpanded macro call
    and emits its own. That one is at least `E0201`.

  So `forward_tool_methods!` generates `call_tool`, `list_tools` and
  `get_tool`; `forward_prompt_methods!` generates `get_prompt` and
  `list_prompts`. Both take an optional page size.

- `pagination`, holding the one cursor implementation the four paginated lists
  share. Previously private to `resources`. Each sequence carries a tag byte,
  so a cursor minted by `tools/list` is rejected by the others — there is a
  test asserting that for every pair.

### Changed

- **Breaking:** a server using `#[tool_handler]` or `#[prompt_handler]` keeps
  working and keeps the old non-paginating behaviour. Moving to the new macros
  means dropping the attribute and writing `get_info` yourself, which
  `server_info` already makes a one-liner.
- `resources::DEFAULT_PAGE_SIZE` is now a re-export from `pagination`. Same
  value, same path.

[#26]: https://github.com/baileyrd/rusty_mcp/issues/26
[#28]: https://github.com/baileyrd/rusty_mcp/issues/28

## [0.4.1] — 2026-08-06

### Fixed

- **Depending on this crate by git printed a cargo error on every build.**
  0.4.0 shipped `template/Cargo.toml` containing `name = "{{project-name}}"`.
  Cargo scans a git checkout for manifests when resolving a git dependency, and
  does so independently of the workspace `exclude` list, so every consumer saw

  ```
  error: invalid character `{` in package name: `{{project-name}}`
  ```

  on every build. The build still succeeded, which is why it went unnoticed —
  it is pure noise in someone else's terminal, and it appears nowhere in this
  repository's own output.

  The template manifest is now named `{{ 'Cargo' }}.toml`. `cargo generate`
  renders Liquid in filenames as well as file contents, so it still arrives as
  `Cargo.toml` in a generated project, while cargo ignores it here. Copying the
  directory by hand now needs that one rename, which the README states.

  `scripts/check-template.sh` asserts `template/Cargo.toml` does not exist, so
  this cannot come back. The workspace `exclude = ["template"]` is dropped: it
  suppressed the error locally while leaving consumers to see it, which is the
  worst of both.

  Anyone on `v0.4.0` should move to `v0.4.1`. Nothing else differs.

### Notes

This was found by checking, after tagging, that the template's own
<!-- check-versions: ignore -->
`tag = "v0.4.0"` dependency actually resolved — the first time that tag
existed to test against. `check-template.sh` cannot catch it by construction, because
it rewrites the dependency to a **path** and so never exercises the git route
that every real consumer takes. That blind spot is now documented in the
README.

## [0.4.0] — 2026-08-06

> **Superseded by [0.4.1]. Use that instead.**
>
> Depending on the `v0.4.0` tag makes cargo print this on every build of your
> project:
>
> ```
> error: invalid character `{` in package name: `{{project-name}}`
> ```
>
> The build still succeeds and the library itself is unaffected, but the noise
> is unavoidable from your side. 0.4.1 fixes it and changes nothing else.

The first release driven by filed issues rather than ad-hoc requests: every
entry below closes one, and two of them changed shape once the code was
examined. Those deviations are recorded here rather than quietly dropped.

### Added

- `completion::CompletionRegistry`, serving `completion/complete` ([#14]).
  `rmcp` ships the wire types but no router, the same gap `ResourceRegistry`
  fills for resources. Registration takes either a fixed list of candidates or
  a closure computing them per request; the closure sees the arguments already
  filled in, which is what makes a dependent completion — a `column` narrowing
  to the chosen `table` — possible.

  Prefix matching (case-insensitive), sorting, the spec's 100-value cap and the
  `hasMore` flag are applied by the registry. `total` reports the count before
  the cap, which is the only way a client can tell it is seeing part of a list.

  `forward_completion_methods!` generates the handler method. An unregistered
  reference or argument completes to an empty list rather than erroring — a
  client asks speculatively, and "nothing to suggest" is the ordinary answer.

- `CompletionRegistry::dangling` and `ResourceRegistry::template_uris`, for
  catching a completion registered against a prompt or template that does not
  exist. That failure is otherwise silent: the registration is accepted and the
  client is answered with an empty list forever.

- `otel::metrics`, exporting metrics alongside spans ([#16]). The same
  `OtelConfig` drives both, so the two share an endpoint and resource;
  `without_metrics()` turns them off.

  `McpMetricsLayer` records `mcp.server.requests`,
  `mcp.server.request.duration` and `mcp.server.requests.in_flight` from the
  SEP-2243 `Mcp-Method`/`Mcp-Name` headers, so no request body is parsed. It
  mounts **outside** the authorization layer, so a request rejected with a
  `401` is still counted — a flood of bad tokens should not look like no
  traffic.

  `TaskSupport::with_metrics` counts `mcp.server.tasks.{started,finished}`.
  Tasks need their own instruments because the work outlives the request that
  handed out the handle, so an HTTP-level layer never sees how one ended.

  Every label comes from a closed set fixed before any request arrives: an
  unknown method is `other`, and a name is recorded only for `tools/call` and
  `prompts/get` and only when it appears in `with_known_names`. The URI in
  `resources/read`'s `Mcp-Name` and the task id in the task methods' are never
  labelled. Calling a tool that does not exist must not be able to mint a
  label; there are tests driving forged names through the layer and asserting
  they never reach the collector.

- `limits::LimitsLayer`, bounding concurrent requests and how long each may
  take ([#17]). Both off by default and set through `HttpConfig::limits`, or
  `--max-concurrent` / `--request-timeout-secs`. Unlike the `Host` allow-list
  there is no default that is right for everyone, and shipping one would be a
  silent regression for any server already handling more.

  Over the limit a request is **shed** with `503` and `Retry-After`, not
  queued. A queue in front of an overloaded server turns a capacity problem
  into a latency problem, which is how a spike becomes an outage. The limit is
  deliberately not applied in `poll_ready`, since returning `Pending` there is
  backpressure, and backpressure means waiting.

  Layer order is now outside-in: **limits → metrics → authorization →
  handler**. A shed request costs a semaphore try-acquire and nothing else,
  rather than paying for a token validation before being told there was no
  capacity for it.

  The timeout bounds time to *produce* a response, not to stream one, so a
  long-lived `subscriptions/listen` is unaffected — verified by a test that
  holds one open for five times the timeout. A tool running inline does hold
  the response open and is cut off with `504`; a tool handed to `TaskSupport`
  releases its permit as soon as the handle goes back to the client, so long
  work never counts against the limit.

  No new dependencies: the permit pool is a `tokio::sync::Semaphore`.

- `template/`, a minimal starting point ([#18]). `cargo generate --git
  https://github.com/baileyrd/rusty_mcp template` produces a compiling server
  with two example tools and a test that drives it through a real client;
  without `cargo generate` it is an ordinary crate to copy.

  The decision recorded on that issue was that a template is only worth having
  if it cannot rot, since it is a second copy of the API surface. So CI
  generates the project and builds, lints and tests it on every pull request —
  **against the working tree rather than the published tag**, so a library
  change that breaks the template fails the pull request that made it.

- `scripts/check-versions.sh`, asserting that every `tag = "vX.Y.Z"` in the
  README, the CHANGELOG preamble and the template names the current crate
  version. This is a regression test for something that already shipped — the
  README told readers to use
  <!-- check-versions: ignore -->
  `rusty-mcp = { git = "...", tag = "v0.2.0", features = ["otel"] }`, a tag
  predating the feature, so following it verbatim did not compile.

### Fixed

- **List results accepted a pagination cursor and ignored it** ([#15]).
  `ResourceRegistry` returned every entry in one response regardless of what
  the client asked for, and never set `next_cursor`. A client that pages saw a
  full first page with no cursor and concluded it had everything, so a large
  registry silently truncated from the client's point of view. Invisible at
  demo scale, which is how it survived twelve releases.

  `list` and `list_templates` now take a cursor and page at
  `DEFAULT_PAGE_SIZE` (100), overridable with `with_page_size`. Cache hints are
  emitted on every page, not just the first.

  The cursor holds a **key, not an index**, and pages are ordered by URI. A
  fabricated cursor therefore names a position in the key space rather than an
  offset into a slice — there is no out-of-range read to guard against — and an
  entry added or deleted between requests cannot shift a page boundary or cause
  an entry to be served twice. A cursor that does not decode, or that was
  minted for the other sequence, is `-32602` rather than an empty page.

### Changed

- **Breaking:** `ResourceRegistry::list` and `list_templates` take
  `Option<&str>` and return `Result`. Servers using
  `forward_resource_methods!` need no change; a hand-written handler calling
  these directly passes `None` to keep the old behaviour, minus the truncation.
- `base64` moves from a dev-dependency to a dependency, for cursor encoding.
- `HttpConfig` gains `limits`, and `metrics` under the `otel` feature. Both
  default to `None`, so an existing config built with `..Default::default()`
  is unaffected; a config built by naming every field needs the new ones.

### Notes

Two issues did not land as filed, and the difference is worth knowing:

- **[#17] claimed there was no request body cap. There already was one** — 4
  MiB by default, `--max-body-bytes`, enforced by `rmcp` incrementally per body
  frame, so an oversized request never gets fully buffered. That was checked
  before implementing, and the issue was corrected rather than a duplicate
  shipped.
- **[#14] asked for registration-time validation of completion references.**
  Delivered as `dangling()` instead, checked at wiring time. The strict version
  would require `CompletionRegistry` to hold references to the prompt router
  and the resource registry, inverting the composition every other registry
  here uses. Same guarantee, one call instead of an inverted dependency.

[#14]: https://github.com/baileyrd/rusty_mcp/issues/14
[#15]: https://github.com/baileyrd/rusty_mcp/issues/15
[#16]: https://github.com/baileyrd/rusty_mcp/issues/16
[#17]: https://github.com/baileyrd/rusty_mcp/issues/17
[#18]: https://github.com/baileyrd/rusty_mcp/issues/18

## [0.3.0] — 2026-08-06

### Added

- `otel` feature: OTLP span export to an OpenTelemetry collector, and
  `TraceContext::attach_parent`, which makes this server's spans genuine
  children of the caller's. 0.2.0 shipped trace-context correlation without an
  exporter; this closes that gap.

  `otel::init` installs a subscriber feeding the pipeline; `otel::pipeline`
  returns the tracer for processes that already build their own. `OtelGuard`
  flushes on shutdown — batched spans are lost otherwise — and
  `OtelGuard::shutdown_hook` wires that into
  `ServerConfig::with_shutdown_hook`.

  Sampling is parent-based, so a caller's decision is honoured rather than
  re-made; export failures are logged and dropped, never surfaced to a caller.

Additive and feature-gated: with `otel` off, the dependency tree and the public
API are exactly 0.2.0's. The minor bump reflects the new `otel` module.

## [0.2.0] — 2026-08-05

### Added

- `mrtr::InputGate`: Multi Round-Trip Requests (SEP-2322), for tools that need
  input from the client mid-call. The server returns an input request and the
  client retries the original call with its answers; since the protocol is
  stateless, the state needed to resume travels through the client in
  `requestState`.

  That value is echoed back verbatim by the client, so `InputGate` seals it
  with HMAC-SHA256, binds it to the tool that created it, expires it, and
  bounds the number of rounds. `Answers::accepted` treats only an explicit yes
  as consent.

  Elicitation gets the typed helpers; sampling and roots are deprecated in this
  revision and remain reachable through the raw `InputRequests` map without
  being encouraged.

### Changed

- `rmcp` now builds with the `request-state` and `elicitation` features. No new
  external dependencies.

Additive only — nothing from 0.1.0 changed shape or was removed. The minor bump
reflects the new public `mrtr` module rather than a break.

## [0.1.0] — 2026-08-05

First tagged release. Everything below shipped together.

### Added

#### Server scaffold ([#1])

- `rusty_mcp::run` and `rusty_mcp::serve`: a handler plus a three-line `main`
  becomes a working MCP server.
- Both transports — stdio and Streamable HTTP — selected by `--transport`.
- `Cli`, with every flag carrying an environment fallback (`MCP_TRANSPORT`,
  `MCP_BIND`, `MCP_PATH`, `MCP_ALLOWED_HOSTS`, …). `RUST_LOG` overrides `--log`.
- Logging to **stderr**, never stdout, since stdout carries framed JSON-RPC on
  the stdio transport.
- Graceful shutdown on `SIGINT`/`SIGTERM`.
- `ServeError` for the runtime, and `ToolError` as a shorthand for building
  `ErrorData` inside tool bodies.
- `server_info()`, which pins the advertised protocol version to 2026-07-28.
  `rmcp`'s `ProtocolVersion::LATEST` still points at `2025-11-25`, so a server
  using `ServerInfo::new` alone advertises the older revision — and never emits
  the cache hints the newer one requires. Older clients still negotiate down.

#### Authorization ([#2], [#4])

- `auth::RequireAuthLayer`, a `tower` layer turning the MCP endpoint into an
  OAuth 2.1 resource server.
- `auth::ProtectedResourceMetadata` (RFC 9728), published **unauthenticated**
  alongside the guarded endpoint — a client that just received a `401` has to be
  able to discover where to authenticate.
- `auth::TokenValidator` for pluggable validation, with `StaticTokenValidator`
  for tests and local development only.
- `auth::JwtValidator` behind the `jwt` feature: JWKS-backed verification of
  signature, expiry, not-before and issuer, with a cached key set. ([#4])
- Per-tool authorization: the verified token is placed in the request
  extensions, reachable from a handler through `http::request::Parts`.

#### Tasks extension ([#3])

- `tasks::TaskSupport`, wrapping `rmcp`'s `TaskManager` with capability
  negotiation, a policy for which tools warrant a task handle, and
  poll-interval/TTL defaults.
- `tasks::TaskCtx`, so one tool body serves both task and inline execution —
  status messages and cancellation degrade to no-ops off-task.
- `forward_task_methods!`, generating `tasks/get`, `tasks/update` and
  `tasks/cancel`.
- `TaskSupport::drain` plus `ServerConfig::with_shutdown_hook`, so in-flight
  tasks get a grace period instead of being dropped mid-step when the process
  stops. `drain` reports how many were abandoned. ([#4])

#### Resources and prompts ([#5])

- `resources::ResourceRegistry`, serving `resources/list`, `resources/read` and
  `resources/templates/list`. `rmcp` has routers for tools and prompts but none
  for resources.
- Three registration shapes: fixed content, content generated per read, and
  templated families via RFC 6570 level-1 URI templates.
- `forward_resource_methods!`, generating the three resource methods.
- Prompts need no new code — `rmcp`'s `PromptRouter` composes like the tool
  routers. Note `prompt_router` takes its router name as a **string literal**
  where `tool_router` takes an identifier.

#### Change notifications ([#8])

- `subscriptions::ChangeBroadcaster`, fanning application change events out to
  every live `subscriptions/listen` subscription. 2026-07-28 replaced the
  standalone HTTP GET stream and `resources/subscribe`/`resources/unsubscribe`
  with this single long-lived request.
- `forward_subscription_methods!`, generating `accepted_subscription_filter`
  and `listen`.
- Publishing is infallible and non-blocking; having no listeners is the normal
  state rather than an error.
- On broadcast lag, accepted list-changed categories are re-announced rather
  than failing the subscription — these are re-fetch signals, so the client ends
  up with fresh lists either way.

#### Tracing ([#6])

- `trace::TraceContext`, parsing and emitting W3C trace context over the bare
  `_meta` keys `traceparent`, `tracestate` and `baggage` (SEP-414).
- `TraceContext::span()`, producing a `tracing` span whose fields carry the
  trace ids so log lines correlate across services.
- `trace::Baggage`, with the W3C limits enforced.

#### Project

- CI running `fmt --check`, `clippy --all-targets --all-features` under
  `-D warnings`, the test suite, and an MSRV (1.88) build.
- 165 tests, including integration coverage over real sockets for both
  transports, authorization, tasks, resources, prompts and trace context.

### Security

These are defaults rather than fixes — no vulnerable version was ever released
— but they are the decisions most worth knowing about:

- **Token audience binding is enforced by the layer, not delegated to the
  validator.** The spec's "MCP servers **MUST NOT** accept or transit any other
  tokens" guards against a confused deputy; a validator that forgot the `aud`
  check would silently reopen it. `VerifiedToken::audience_checked_by_validator`
  is the explicit opt-out.
- **JWT algorithms are checked against an allow-list before any key is
  fetched**, which is what blocks `alg: none` and the RS256-verified-as-HS256
  family. RS256 and ES256 only by default.
- **JWKS refetches provoked by an unknown `kid` are rate-limited**, so random
  `kid` values cannot drive unbounded outbound requests to the authorization
  server.
- **Resource template variables never match across `/`**, so
  `db://tables/{table}` does not match `db://tables/../../etc/passwd`. Values
  are percent-decoded after matching, so decoding cannot reintroduce a
  separator.
- **Streamable HTTP defaults to a loopback-only `Host` allow-list**, guarding
  local servers against DNS rebinding. Public deployments must set their own.
- **Baggage is treated as untrusted input**, with the W3C caps (180 entries,
  8 KiB) enforced. It crosses service boundaries unauthenticated and must never
  inform an authorization decision.
- **Neither crate is publishable.** Both carry `publish = false`, so cargo
  refuses rather than relying on nobody typing `cargo publish`.

### Notes on protocol behaviour

Details of 2026-07-28 that changed how this is built, recorded because they are
easy to get wrong:

- The protocol is **stateless**: no `initialize` handshake, no
  `Mcp-Session-Id`, no stream resumption. Streamable HTTP uses
  `NeverSessionManager` by default, so no sessions are minted at all;
  `--legacy-sessions` opts back in for pre-2026-07-28 clients.
- **Resource-not-found changed from `-32002` to `-32602`.** `rmcp` remaps by
  negotiated protocol version, so its constructor is correct — but a hardcoded
  number would not be.
- **Cache hints (`ttlMs`, `cacheScope`) are required** on `tools/list`,
  `prompts/list`, `resources/list`, `resources/read` and
  `resources/templates/list`.
- **A task handle may only go to a client that declared the tasks extension**;
  sending one otherwise is rejected with `-32021` rather than degrading.
- Roots, Sampling and Logging are **deprecated** in this revision with a
  12-month window. None are implemented here, deliberately.
- **Subscription filters are intersected with advertised capabilities.** A
  category the server does not advertise is dropped without error, so the
  subscription succeeds and stays quiet — advertise the `list_changed` flags
  for anything you intend to send.

[#1]: https://github.com/baileyrd/rusty_mcp/pull/1
[#2]: https://github.com/baileyrd/rusty_mcp/pull/2
[#3]: https://github.com/baileyrd/rusty_mcp/pull/3
[#4]: https://github.com/baileyrd/rusty_mcp/pull/4
[#5]: https://github.com/baileyrd/rusty_mcp/pull/5
[#6]: https://github.com/baileyrd/rusty_mcp/pull/6
[#8]: https://github.com/baileyrd/rusty_mcp/pull/8
[0.4.1]: https://github.com/baileyrd/rusty_mcp/releases/tag/v0.4.1
[0.4.0]: https://github.com/baileyrd/rusty_mcp/releases/tag/v0.4.0
[0.3.0]: https://github.com/baileyrd/rusty_mcp/releases/tag/v0.3.0
[0.2.0]: https://github.com/baileyrd/rusty_mcp/releases/tag/v0.2.0
[0.1.0]: https://github.com/baileyrd/rusty_mcp/releases/tag/v0.1.0
[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html
[spec]: https://modelcontextprotocol.io/specification/2026-07-28
[rmcp]: https://crates.io/crates/rmcp
