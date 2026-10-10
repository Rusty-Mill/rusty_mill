# Making `rusty-mcp` sovereign

Scope: the `rusty-mcp` crate (`crates/libs/protocol/rusty_mcp/crates/rusty-mcp`,
about 3.7k lines) after A6's `rmcp` half. It is the gateway-support crate:
`auth`, `limits`, `otel`, `trace`. This is step A6, second half, of
`MCP-NATIVE-PLAN.md`: replace what is still external with first-party crates.
Facts below were read from the code on 2026-10-10; nothing here is built yet.

## 1. What is still external, and what replaces it

| External | Used for (files) | First-party replacement | Gap | Size |
|---|---|---|---|---|
| `percent-encoding` | baggage values (`trace.rs`) | `rusty_percent` | none | trivial |
| `serde`, `serde_json` | `Value` in `trace`, `auth/*`; `Serialize` on `ProtectedResourceMetadata` | `rusty_json` (`Value`, hand-written codec, as `rusty_mcp_proto` does) | metadata needs a `to_value`; gateway callers pass `serde_json::Value` today | small |
| `thiserror` | error enums in `auth/*`, `otel/mod.rs` | `rusty_err` (derive crate) | check the derive covers `#[error("..")]`, `#[from]`, `#[source]` | small |
| `reqwest` | JWKS fetch (`auth/jwt.rs`, feature `jwt`) | `rusty_request` (+ `rusty_tls`) | `rusty_request` is async on `rusty_tokio`; the validator runs inside the gateway's `tokio`. Needs a bridge (own runtime, as `rusty_mcp_client_native` does) | medium |
| `jsonwebtoken` | RS256 and ES256 verify, claim checks, JWKS key lookup (`auth/jwt.rs`, 475 lines) | `rusty_oauth`: `jwt::rsa::verify_rs256`, `jwt::es256::verify_es256`, `jwks::JwkSet` (`rsa_key`, `ec_key`), `jwt::{decode_unverified, validate_claims}`; primitives in `rusty_pk`, `rusty_sha2` | glue only. Must keep: algorithm whitelist (no `none`, no HS with a public key), `kid` lookup with refetch limits, `iss`/`aud` (array form)/`exp`/`nbf` with leeway. Reuse the 8 `jwt_validator` tests as the parity suite | medium |
| `axum` | `Response`/`IntoResponse` in `auth/layer.rs`, `limits.rs`, `otel/metrics.rs`; `Json` in doc examples | none needed: build `http::Response<B>` with a generic body | a layer that wraps an `axum::Router` must return axum's body type, so be generic over `B: From<...>` | medium |
| `tower-layer`, `tower-service` | the `Layer`/`Service` impls of all three layers | none possible while the gateway is axum | interface-only crates (about 100 lines, no dependencies). Keep as a named exception until the gateway leaves axum | exception |
| `http` | request/response types | `rusty_http` | the gateway speaks `http` types through axum. Keep at the adapter boundary only | exception |
| `tokio` | `Semaphore`, `RwLock`, `time` in `limits`, `auth/jwt`, `auth/token` | `rusty_tokio`, or `std` atomics | `limits` needs no semaphore (a CAS counter is enough); the timeout needs a runtime timer, so the tower adapter keeps `tokio::time` | small, then exception |
| `tracing`, `tracing-subscriber` | spans in `trace`/`layer`/`limits`; subscriber setup in `otel/mod.rs` | none in-house | the log facade is used workspace-wide (74 crates). A decision, not a task (D3) | decision |
| `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry` | OTLP trace and metrics pipeline, `McpMetricsLayer` instruments (`otel/mod.rs` 418, `otel/metrics.rs` 551, `trace.rs`) | a first-party OTLP exporter: instruments as atomics, spans from a `tracing` `Layer`, encoding with `rusty_json` (OTLP/HTTP+JSON) over `rusty_request` | no in-house OTLP, no protobuf, no gRPC client. This is the largest item (D2) | large |
| already first-party | `rusty_url`, `rusty_base64` | | | done |

## 2. Decisions needed first

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
   unreachable-JWKS and refetch-limit tests pass.
4. **JWT verification on `rusty_oauth`**, replacing `jsonwebtoken`. Port the
   algorithm whitelist, claim checks and `kid` handling; add negative tests for
   `alg: none`, an HS256 token signed with the public key, wrong `aud`, expired,
   not-yet-valid, unknown `kid`. *Done when:* `jsonwebtoken` is gone and the 8
   existing validator tests plus the new negatives pass. Needs D5.
5. **Sans-IO cores** for `auth`, `limits` and metrics recording, on `rusty_http`
   types; the existing layers become tower adapters over them. No behaviour
   change. *Done when:* the cores have unit tests with no `tokio` or `axum`.
6. **Drop `axum`** from the adapters (generic body, `http::Response`). *Done
   when:* `axum` is gone from `[dependencies]`; the gateway still compiles and
   its 24 suites pass. Needs D1.
7. **`rusty_serve` adapter** for the cores, so a blocking server can use auth
   and limits without tower. One test per layer against `rusty_serve`.
8. **`limits` without `tokio::sync`**: an atomic admission counter; keep
   `tokio::time` only in the tower adapter's timeout.
9. **OTLP exporter**, in three PRs: (a) instruments and a span model on atomics
   with an exporter trait; (b) the OTLP/HTTP-JSON encoder and sender on
   `rusty_request`, tested against the fake collector the tests already have;
   (c) a `tracing` `Layer` feeding spans, replacing `tracing-opentelemetry`.
   *Done when:* the four OpenTelemetry crates are gone; the `otel_export` and
   `otel_metrics` tests pass; the cardinality guard still holds on the wire.
   Needs D2 and D3.
10. **Policy and docs.** `check_workspace_deps.py` accepts `rusty-mcp` as
    Tier S except the named exceptions (`http`, `tower-layer`, `tower-service`,
    `tokio` in the adapter, `tracing` if D3 says keep). ADR-0002 Amendment 1
    status line and `MCP-NATIVE-PLAN.md` A6 updated.

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
