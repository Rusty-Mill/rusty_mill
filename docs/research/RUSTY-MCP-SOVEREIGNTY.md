# Making `rusty-mcp` sovereign

Scope: the `rusty-mcp` crate (`crates/libs/protocol/rusty_mcp/crates/rusty-mcp`,
about 3.7k lines) after A6's `rmcp` half. It is the gateway-support crate:
`auth`, `limits`, `otel`, `trace`. This is step A6, second half, of
`MCP-NATIVE-PLAN.md`: replace what is still external with first-party crates.
Facts below were read from the code on 2026-10-10, before any of it was built; the ordered list in section 3 records what is done.

## 1. What is still external, and what replaces it

| External | Used for (files) | First-party replacement | Gap | Size |
|---|---|---|---|---|
| `percent-encoding` | baggage values (`trace.rs`) | `rusty_percent` | none | done |
| `serde`, `serde_json` | `Value` in `trace`, `auth/*`; `Serialize` on `ProtectedResourceMetadata` | `rusty_json` (`Value`, hand-written codec, as `rusty_mcp_proto` does) | metadata needs a `to_value`; gateway callers pass `serde_json::Value` today | done (direct dependency) |
| `thiserror` | error enums in `auth/*`, `otel/mod.rs` | hand-written `Display` and `std::error::Error` impls, not `rusty_err` | `rusty_err`'s derive implements its own `Error` trait, not `std::error::Error`, so `?` into `Box<dyn std::error::Error>` (the gateway) would break. Four small enums, no dependency | done |
| `reqwest` | JWKS fetch (`auth/jwt.rs`, feature `jwt`) | `rusty_request` with its `tokio` feature, which runs each request on the caller's own runtime | none: no bridge or second runtime needed (the plan expected one). `reqwest` stays as a dev-dependency of the tests | done |
| `jsonwebtoken` | RS256 and ES256 verify, claim checks, JWKS key lookup (`auth/jwt.rs`, 475 lines) | `rusty_oauth`: `jwt::rsa::verify_rs256`, `jwt::es256::verify_es256`, `jwks::JwkSet` (`rsa_key`, `ec_key`), `jwt::decode_unverified`; MCP checks claims locally. RSA uses `rusty_rsa::BigUint`; ES256 uses `rusty_oauth::crypto::ecc` and that same bigint; SHA-256 is re-exported from `rusty_rsa` (not `rusty_pk`/`rusty_sha2`) | glue only. Must keep: algorithm whitelist (no `none`, no HS with a public key), `kid` lookup with refetch limits, `iss`/`aud` (array form)/`exp`/`nbf` with leeway. Reuse the 8 `jwt_validator` tests as the parity suite | done (dev-dependency only, to sign test tokens) |
| `axum` | `Response`/`IntoResponse` in `auth/layer.rs`, `limits.rs`, `otel/metrics.rs`; `Json` in doc examples | none needed: build `http::Response<B>` with a generic body | a layer that wraps an `axum::Router` must return axum's body type, so be generic over `B: From<...>` | medium |
| `tower-layer`, `tower-service` | the `Layer`/`Service` impls of all three layers | none possible while the gateway is axum | interface-only crates (about 100 lines, no dependencies). Keep as a named exception until the gateway leaves axum | exception |
| `http` | request/response types | `rusty_http` | the gateway speaks `http` types through axum. Keep at the adapter boundary only | exception |
| `tokio` | `Semaphore`, `RwLock`, `time` in `limits`, `auth/jwt`, `auth/token` | `rusty_tokio`, or `std` atomics | `limits` needs no semaphore (a CAS counter is enough); the timeout needs a runtime timer, so the tower adapter keeps `tokio::time` | small, then exception |
| `tracing`, `tracing-subscriber` | spans in `trace`/`layer`/`limits`; subscriber setup in `otel/mod.rs` | none in-house | the log facade is used workspace-wide (74 crates). A decision, not a task (D3) | decision |
| `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry` | OTLP trace and metrics pipeline, `McpMetricsLayer` instruments (`otel/mod.rs` 418, `otel/metrics.rs` 551, `trace.rs`) | a first-party OTLP exporter: instruments as atomics, spans from a `tracing` `Layer`, encoding with `rusty_json` (OTLP/HTTP+JSON) over `rusty_request` | no in-house OTLP, no protobuf, no gRPC client. This was the largest item (D2) | done (2026-10-18): metrics and spans are first-party and the four crates are gone from `rusty-mcp` |
| already first-party | `rusty_url`, `rusty_base64` | | | done |

## 2. Decisions

Answered by the owner 2026-10-10: D1 cores plus adapters; D2 OTLP/HTTP+JSON;
D3 keep `tracing` now, in-house facade is item 12; D4 yes, exceptions
accepted; D5 parity first, other algorithms are item 11. The text below is the
reasoning behind each.

- **D1. Adapter shape.** Recommended: sans-IO cores (authorize a request,
  admit or shed, record a metric) plus two thin adapters: a `tower` layer
  (for the axum gateway) and a `rusty_serve` `Handler` wrapper (for blocking
  servers). The cores depend on `rusty_http` and `rusty_json` only. The
  alternative, tower only, keeps `axum`-shaped code alive in a sovereign crate.
- **D2. OTLP transport.** The current pipeline is OTLP/gRPC (`tonic`, `prost`,
  `h2`). In-house there is `rusty_h2` but no protobuf or gRPC client.
  Recommended: OTLP/HTTP with the JSON encoding (spec-defined, accepted by the
  OpenTelemetry Collector on 4318), built on `rusty_json` and `rusty_request`.
  Cost: the default endpoint and port change, and a backend that only takes
  gRPC needs a collector in front. Protobuf-over-HTTP can come later (a small
  hand-written encoder on `rusty_wire`).
- **D3. `tracing` facade.** Keep `tracing` and `tracing-subscriber` as Tier A
  (workspace-wide logging, not specific to MCP) and replace only the
  OpenTelemetry crates; or build an in-house facade (a workspace-wide change
  that is out of MCP's scope). Recommended: keep.
- **D4. Gateway and axum.** `agentgateway` is an axum app, as are 12 other
  crates (`RUSTY-AXUM-SCOPE.md`). `rusty-mcp` can become axum-free first (D1),
  but `tower-*`, `http` and `tokio` stay in the tower adapter until a gateway
  decision is made. Is that acceptable as the end state of this track?
- **D5. JWT scope.** Keep RS256 and ES256 only (today's default and what the
  tests cover), or add PS256, ES384, EdDSA now that `rusty_pk` has the
  primitives? Recommended: parity first.

## 3. The list, in order

Each item is one PR, with its own tests and a change fragment.

1. **Trivial swaps.** `percent-encoding` to `rusty_percent`; `thiserror` to
   `rusty_err`. *Done when:* both crates are gone from `Cargo.toml`, tests
   unchanged. **Done (2026-10-10), with one change:** `thiserror` went to
   hand-written `Display` and `std::error::Error` impls, not `rusty_err`,
   whose derive implements `rusty_err::Error` and not `std::error::Error`
   (it cannot, because of its blanket impl), which would break `#[source]`
   and `?` in the gateway crates. Baggage no longer escapes `-_.~`.
2. **`serde`/`serde_json` to `rusty_json`** across `auth` and `trace`.
   `ProtectedResourceMetadata::to_value`. The gateway call sites move with it.
   *Done when:* no `serde` in the crate; the metadata document is
   byte-identical in the existing authorization test. **Done (2026-10-10),
   with three changes:** (1) `rusty-mcp` lists no `serde` or `serde_json`, but
   `serde` stays in its tree through `jsonwebtoken` (until item 4) and
   `rusty_json`'s default `serde` feature, which `decode::<Value>` and axum's
   `Json` need until items 4 and 5. (2) The metadata document has the same
   content, not the same bytes: keys are now alphabetical, not in declaration
   order (the existing test parses the body, so it never compared bytes).
   (3) `VerifiedToken::claims` is a `rusty_json::Value`; the gateway converts
   once, in `TokenClaims::from_json`. Its rules and `agentgateway-llm` stay on
   `serde_json`, which is the gateway's own track, not this crate's.
3. **JWKS fetch on `rusty_request`**, replacing `reqwest`, with a bridge that
   works inside a `tokio` caller. *Done when:* `reqwest` is gone; the
   unreachable-JWKS and refetch-limit tests pass. **Done (2026-10-11), with
   three changes:** (1) No bridge: `rusty_request`'s additive `tokio` feature
   picks the runtime per call, so the fetch runs on the gateway's own `tokio`
   and no second runtime starts; its own tests cover HTTP and HTTPS that way.
   (2) `reqwest` is gone from `[dependencies]` but stays in
   `[dev-dependencies]`, because the integration tests use it as a client for
   their axum servers; `agentgateway-auth` keeps its own `reqwest` for
   `extauthz`. (3) `JwtValidatorError::Http` is removed: building a
   `rusty_request` client cannot fail, so the variant could never be
   produced. `rusty-mcp` is 0.8.0 for that API change. (4) Behaviour that
   differs from `reqwest`, found in review: no connection pool (a stale pooled
   connection is an error in `rusty_request`, not retried), a 1 MiB cap on the
   JWKS body (since raised to 4 MiB, per client, because Bot Framework's key set is
   already about 910 KB), and HTTPS trust from `rusty_tls`'s system anchors, which on
   macOS ignores Keychain trust and distrust settings (rustils#88). Interim
   `1xx` responses are now handled in `rusty_request`.
4. **JWT verification on `rusty_oauth`**, replacing `jsonwebtoken`. Port the
   algorithm whitelist, claim checks and `kid` handling; add negative tests for
   `alg: none`, an HS256 token signed with the public key, wrong `aud`, expired,
   not-yet-valid, unknown `kid`. *Done when:* `jsonwebtoken` is gone and the 8
   existing validator tests plus the new negatives pass. Needs D5. **Done
   (2026-10-11), with four changes:** (1) `jsonwebtoken` is gone from
   `rusty-mcp`'s `[dependencies]`; it stays as a dev-dependency because
   `rusty_oauth` only verifies RSA signatures, so the tests still sign RS256
   tokens with it, and `agentgateway-auth` keeps its own copy for `file_jwks`.
   (2) The public `with_algorithms` takes the new `JwtAlgorithm` (`Rs256`,
   `Es256`, `#[non_exhaustive]`) instead of `jsonwebtoken::Algorithm`, so
   `rusty-mcp` is 0.9.0. (3) Stricter than before, on purpose: RSA moduli
   must be 2048 to 4096 bits (the old verifier enforced only the upper bound),
   two JWKS keys with one `kid` match nothing, base64 must be canonical,
   and leeway is capped at `i32::MAX` seconds. Odd RSA moduli and odd
   exponents in 3..=2^33-1 were already required by the former backend;
   P-256 JWK coordinates retain their required 32-byte width.
   (4) "Wrong `aud`" has no validator test:
   the validator leaves `aud` to the layer by design, and the layer's tests
   cover it. `rusty_oauth`'s RSA verification had no key-size bounds of its
   own, so the bounds first lived in this crate; they now live in `rusty_oauth`
   (key-bounds PR) and this crate asks it whether a key is usable.
   **Review corrections (2026-10-12):** MCP checks NumericDate locally:
   unsigned integers (including values above i64::MAX) remain exact; finite
   nonnegative floats below 2^64 round to seconds, matching jsonwebtoken 11.
   Present malformed `exp`/`nbf` fail closed, while absent `nbf` is allowed.
   Widened arithmetic prevents overflow; expiry at exactly `now - leeway`
   remains accepted as before. Signed regressions cover these forms, and
   independent OpenSSL ES256 integration covers valid signatures plus invalid
   scalars/keys. The underlying primitives are `rusty_rsa::BigUint` and
   `rusty_oauth::crypto::ecc`, with SHA-256 from `rusty_rsa`; validation evidence
   for `rusty_pk` does not transfer to these implementations.
5. **Sans-IO cores** for `auth`, `limits` and metrics recording, on `rusty_http`
   types; the existing layers become tower adapters over them. No behaviour
   change. *Done when:* the cores have unit tests with no `tokio` or `axum`.
   **`limits` core done (2026-10-14):** crate-private `admission` (`Admission`,
   `Permit`, `Rejection`); `auth` and metrics remain.
   **Metrics core done (2026-10-14):** crate-private `otel::labels` (the
   cardinality guard and per-instrument attribute sets), under the `otel` feature.
   **`auth` core done (2026-10-14):** crate-private `auth::decision`
   (`bearer_token`, `from_validation`, `check_claims`, `Refusal`). With this, all three
   cores are done.
6. **Drop `axum`** from the adapters (generic body, `http::Response`). *Done
   when:* `axum` is gone from `[dependencies]`; the gateway still compiles and
   its 24 suites pass. Needs D1.
   **Done (2026-10-18):** `LimitsLayer`, `RequireAuthLayer` and `McpMetricsLayer`
   are generic over the response body (`Service<Request<_>, Response =
   http::Response<B>>`, with `B: From<String>` where the layer writes a
   refusal). `axum` moved to `[dev-dependencies]`, for the tests that mount the
   layers on a router. The gateway needed no change; its suites pass. Breaking:
   a service that returned anything but an `http::Response` (an `IntoResponse`
   tuple) no longer fits; `rusty-mcp` is 0.11.0.
7. **`rusty_serve` adapter** for the cores, so a blocking server can use auth
   and limits without tower. One test per layer against `rusty_serve`.
   **Done (2026-10-15)**, behind a `serve` feature: `serve::LimitedHandler` and
   `serve::AuthorizedHandler` for a `SharedHandler`. Limits have no timeout (a
   blocking handler cannot be cancelled) and the validator runs on a `std`
   executor with no reactor, so the JWKS-backed validator needs the tower layers.
8. **`limits` without `tokio::sync`**: an atomic admission counter; keep
   `tokio::time` only in the tower adapter's timeout. **Done (2026-10-14)**
   with the `limits` core: `Admission` is a compare-and-swap counter.
9. **OTLP exporter**, in three PRs: (a) instruments and a span model on atomics
   with an exporter trait; (b) the OTLP/HTTP-JSON encoder and sender on
   `rusty_request`, tested against the fake collector the tests already have;
   (c) a `tracing` `Layer` feeding spans, replacing `tracing-opentelemetry`.
   *Done when:* the four OpenTelemetry crates are gone; the `otel_export` and
   `otel_metrics` tests pass; the cardinality guard still holds on the wire.
   Needs D2 and D3.
   **Done (2026-10-18), by signal rather than (a)/(b)/(c):** PR 1, metrics
   (atomic instruments, OTLP/HTTP JSON, push thread on `rusty_request`);
   PR 2, spans (`SpanLayer` replacing `tracing-opentelemetry`, a batching
   exporter on the same worker, `otel::set_attribute`), which also removed the
   four crates and the SDK-typed public API, since nothing used them once
   spans moved; PR 3, the `rusty-mcp` 0.10.0 bump and item 10.
10. **Policy and docs.** **Done (2026-10-18).** `check_workspace_deps.py` did
    not classify tiers (it only failed on git copies of workspace members;
    ADR-0002 leaves a tier ledger as follow-up), so there was no Tier S check to
    teach about `rusty-mcp`. What it now does is pin the crate's direct external
    dependencies (normal and build, any feature; dev is exempt) to a list with a
    reason per entry: `http`, `tower-layer`, `tower-service`, `tokio`
    (the tower adapter's timeout); `tracing` and `tracing-subscriber` left this list on 2026-10-19 when `rusty-mcp` moved onto `rusty_log` (item 12).
    A dependency off the list fails the `dependency policy` CI job, and so does
    an entry the crate no longer uses, so the list only shrinks: item 6 deleted
    `axum` (2026-10-18), item 12 deleted the two `tracing` lines for `rusty-mcp` (2026-10-19). ADR-0002 Amendment 1's
    status line and `MCP-NATIVE-PLAN.md` A6 are updated. `rusty-mcp` is 0.10.0
    for item 9's breaking changes (OTLP over HTTP and its endpoints, the
    SDK-typed API removed); items 5, 7 and 8 kept the public API or only added
    to it.
11. **More JWT algorithms** (after item 4): PS256, ES384, EdDSA, one PR each,
    on the `rusty_pk` primitives. Same negative-test bar as item 4, plus the
    algorithm whitelist stays explicit (nothing is accepted by default).
12. **In-house `tracing` facade** (workspace-wide, after item 9): replace
    `tracing` and `tracing-subscriber` in the 74 crates that use them. Its own
    plan and owner sign-off before any code; until then `tracing` is a named
    Tier A exception of `rusty-mcp`.
    **Done (2026-10-19), as option B of `TRACING-FACADE-PLAN.md`:** a `rusty_log`
    re-export crate, all 31 workspace crates that used `tracing` or `tracing-subscriber`
    migrated onto it (one PR per area), and a policy check that only `rusty_log` may
    depend on `tracing*` directly. The third-party crates in the graph (`axum`, `h2`,
    `sqlx`, `aws-*`) still depend on `tracing`, so the crate stays in the build graph;
    an in-house implementation (option C) is not planned.

## 4. Risks

- **Crypto.** Replacing a widely used JWT crate with in-house verification is
  the riskiest step. Verification uses public keys only, so there is no secret
  to leak, but a missed check (algorithm confusion, `aud` as an array) is an
  authentication bypass. Treat item 4 as security-sensitive: negative tests
  first, and a second reviewer.
- **Telemetry fidelity.** A homemade exporter will not match every backend. The
  fake-collector tests prove the bytes reach the wire, not that a particular
  backend accepts them; check one real collector before relying on it.
- **Runtime mixing.** `rusty_request` runs on `rusty_tokio`; the gateway runs on
  `tokio`. The bridge in item 3 must not block a `tokio` worker.
- **Public API.** Items 2, 5, 6 and 9 change public types; each is a breaking
  bump of `rusty-mcp` and a migration of three gateway crates.

## 5. Not in this list

- Replacing `axum` in the gateway itself, and the other 12 axum crates
  (`RUSTY-AXUM-SCOPE.md`).
- The TLS track (`rusty_tls` native engine as default).
- Nexus, which is frozen and outside the workspace.
