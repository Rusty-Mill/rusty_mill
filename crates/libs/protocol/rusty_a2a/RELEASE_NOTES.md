# Release Notes

This repo has no version tags yet, so entries track merged PRs against `main`
instead, reverse chronological, each linking to its PR.

---

## PR #30 — Fix stale doc comments describing the crate as JSON-RPC-only
**2026-08-08** · [#30](https://github.com/baileyrd/rusty_a2a/pull/30)

- **Fixed:** an audit of every module-level doc comment against current
  code found several left over from before the REST/gRPC bindings
  existed - `src/server/mod.rs` and `src/client/mod.rs`'s crate-level
  docs both still described the crate as JSON-RPC-only, when the server
  actually serves JSON-RPC + REST unconditionally (gRPC on top via
  `AgentServices`) and the client module has held `RestClient`/
  `GrpcClient` alongside `A2aClient` for a while. Also fixed two
  `specification/a2a.proto` path references that should have read
  `spec/a2a.proto` (this crate's actual vendored path), and a wrong spec
  section citation for Agent Card signing in `ARCHITECTURE.md` (was 6,
  is 8.4) introduced in #29.
- **Known limitation, stated plainly:** `cargo doc --features <partial>`
  (anything short of `full`) already had unresolved intra-doc-link
  errors before this change, for unrelated pre-existing reasons (feature-
  gated items referenced unconditionally elsewhere in the crate) - left
  alone here since CI's `Docs` job only ever runs `--features full`,
  which is clean.
- No code changes; doc comments and `ARCHITECTURE.md` only.

## PR #29 — Apply standard governance-file scaffolding
**2026-08-08** · [#29](https://github.com/baileyrd/rusty_a2a/pull/29)

- **Added:** the standard repo-config file set - PR/issue templates,
  `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CHANGELOG.md`,
  this file, `ARCHITECTURE.md` (hand-adapted with a real ports/adapters
  table and data-flow walkthrough), and an ADR log seed.
- **Known limitation, stated plainly:** the template `ci-rust.yml` was
  deliberately skipped - this repo's existing `.github/workflows/ci.yml`
  already gates on fmt/clippy/test/doc plus 9 feature-combination checks
  and installs `protoc` for the `grpc` feature, which the generic template
  doesn't do, so adding it would have introduced a redundant, always-broken
  second workflow rather than closing a real gap.

## PR #28 — Warn at startup when an apiKey query/cookie scheme meets a non-REST binding
**2026-08-08** · [#28](https://github.com/baileyrd/rusty_a2a/pull/28)

- **Added:** `Engine::new` now logs a `tracing::warn!` when the `AgentCard`
  declares an `apiKey` security scheme located at `query`/`cookie` alongside
  a non-REST interface (JSON-RPC or gRPC). The A2A spec requires equivalent
  authentication across every declared binding (Section 5.1) but only ever
  defines header/metadata credential transport for JSON-RPC/gRPC (Sections
  7.3/9.2/10.2) — such a scheme is realistically satisfiable only over REST,
  so this is a loud diagnostic for a likely-unintended `AgentCard`, not a
  rejection, since the spec doesn't actually forbid the combination.
- **Known limitation, stated plainly:** this is diagnostic-only; nothing
  rejects the request itself, since the spec leaves this an open gap rather
  than a defined error condition.
- 4 new unit tests in `src/server/engine.rs`.

## PR #27 — Add per-principal task authorization scoping (spec Section 13.1)
**2026-08-07** · [#27](https://github.com/baileyrd/rusty_a2a/pull/27)

- **Added:** `AuthVerifier::authorize_task`, a new trait method (no-op
  default, so existing verifiers are unaffected) called for every operation
  that touches a specific task once a request has authenticated — letting a
  consumer's `AuthVerifier` decide whether the authenticated caller may
  access that particular task. `ListTasks` calls it per candidate task and
  silently omits whatever it rejects rather than failing the whole call, per
  the spec's "MUST only return tasks visible to the authenticated client."
- **Known limitation, stated plainly:** this crate has no opinion on what
  "authorized" means for a given deployment (ownership? role-based?
  team-shared?) — same reason it doesn't decide what a valid credential is.
- 6 new integration tests in `tests/task_authorization_scoping.rs`, including
  a pagination-correctness case across multiple internal store pages.

## PR #26 — Close eleven gaps from the third A2A spec-compliance audit
**2026-08-07** · [#26](https://github.com/baileyrd/rusty_a2a/pull/26)

- **Fixed:** eleven spec-compliance gaps found by a full audit across every
  section of the A2A spec, including: `GetExtendedAgentCard` now fails closed
  instead of silently serving the card unauthenticated when misconfigured;
  `pageSize`/`historyLength` validation errors instead of silent clamping;
  the JSON-RPC binding now distinguishes `-32700` (invalid JSON) from
  `-32600` (valid JSON, invalid Request object); webhook URLs must be
  `https` under SSRF protection; JWS-signed Agent Cards always carry `typ`
  and a derived `kid`; and several `oneof`-shaped types (`PartContent`,
  `SecurityScheme`, `OAuthFlows`) now reject ambiguous JSON instead of
  silently resolving to whichever variant matches first.
- 3 new integration test files; several existing suites extended.

<!--
New entries go above this line, newest first. Bolded category tag inline in
the bullet (**Added:** / **Changed:** / **Fixed:**), reasoning included, and
known limitations or deliberate scope cuts stated plainly rather than left
implied — see PR #27/#28 above for the shape to match.
-->
