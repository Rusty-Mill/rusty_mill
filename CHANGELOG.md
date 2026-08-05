# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog][kac], and the project aims to follow [Semantic
Versioning][semver] once it starts tagging releases.

**Nothing has been released yet.** There are no tags, and neither crate is
published to crates.io — consume `rusty-mcp` via a git or path dependency. The
`0.1.0` in `Cargo.toml` is a placeholder, so everything built so far sits under
Unreleased rather than being back-dated into a version that never shipped.

Targets MCP specification [2026-07-28][spec], on [`rmcp`][rmcp] 3.x.

## [Unreleased]

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

#### Tracing ([#6])

- `trace::TraceContext`, parsing and emitting W3C trace context over the bare
  `_meta` keys `traceparent`, `tracestate` and `baggage` (SEP-414).
- `TraceContext::span()`, producing a `tracing` span whose fields carry the
  trace ids so log lines correlate across services.
- `trace::Baggage`, with the W3C limits enforced.

#### Project

- CI running `fmt --check`, `clippy --all-targets --all-features` under
  `-D warnings`, the test suite, and an MSRV (1.88) build.
- 149 tests, including integration coverage over real sockets for both
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

[#1]: https://github.com/baileyrd/rusty_mcp/pull/1
[#2]: https://github.com/baileyrd/rusty_mcp/pull/2
[#3]: https://github.com/baileyrd/rusty_mcp/pull/3
[#4]: https://github.com/baileyrd/rusty_mcp/pull/4
[#5]: https://github.com/baileyrd/rusty_mcp/pull/5
[#6]: https://github.com/baileyrd/rusty_mcp/pull/6
[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html
[spec]: https://modelcontextprotocol.io/specification/2026-07-28
[rmcp]: https://crates.io/crates/rmcp
