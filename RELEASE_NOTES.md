# Release Notes

This repo has no version tags yet, so entries track merged PRs against `main`
instead, reverse chronological, each linking to its PR.

---

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
