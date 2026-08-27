# Gap analysis: rusty_tls vs. ARCHITECTURE.md's Non-goals

Scope for this run (per user decision): every item in `ARCHITECTURE.md`'s
**Non-goals** section is treated as in-scope, overriding the repo's normal
consumer-gating discipline for this round. "Hand-rolled cryptography" is
excluded below — it's a permanent policy ("rustls stays the engine... never
silently promoted to default"), not a capability gap to close.

Source for every row is `roadmap`: audited against `ARCHITECTURE.md`'s
existing hand-curated Non-goals list, not an independently-invented scope.

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `AsyncTlsServerStream` | type | roadmap | both | ARCHITECTURE.md Non-goals: "async server adapter" | no | M | Mirrors `TlsServerStream` (server) crossed with `AsyncTlsStream`'s poll-driven I/O (async): drives `rustls::ServerConnection` over `rusty_tokio`, behind the existing `rusty-tokio` feature. No confirmed consumer (Non-goals text: rusty_llama's server-TLS shape "has never been verified against its actual source") — implemented speculatively per this run's scope decision. |
| `TlsStream`/`AsyncTlsStream` client identity (mTLS, client side) | fn (new ctor) | roadmap | both | ARCHITECTURE.md Non-goals: "client-certificate authentication (mTLS)" | no | M | Additive: new constructor (e.g. `new_with_client_identity`) taking a client cert chain + private key DER, alongside the existing `new`. Covers both the sync and async client adapters — split into two issues if scope creeps during implementation. |
| `TlsAcceptor` client-cert verification (mTLS, server side) | fn (new ctor) | roadmap | both | ARCHITECTURE.md Non-goals: "client-certificate authentication (mTLS)" | no | M | Additive: new constructor (e.g. `new_with_client_auth`) taking a trusted client-CA root store; today's `with_no_client_auth()` path stays the default via the existing `TlsAcceptor::new`. |
| ALPN negotiation (client + server) | fn (new ctor) | roadmap | both | ARCHITECTURE.md Non-goals: "ALPN" | no | S | Additive: new constructors accepting `alpn_protocols: Vec<Vec<u8>>` on `TlsStream`/`AsyncTlsStream`/`TlsAcceptor`, plus a new accessor to read back the negotiated protocol. |
| Session resumption (client + server) | fn (existing, internal) | roadmap | both | ARCHITECTURE.md Non-goals: "session resumption" | no | M | Today a fresh `ClientConfig`/`ServerConfig` — and thus a fresh resumption cache — is built on every `TlsStream::new`/`TlsAcceptor::new` call, so rustls' built-in resumption never actually triggers regardless of feature support. Needs a caller-visible way to reuse config/cache across connections for the same policy; design that reuse handle deliberately so it doesn't silently turn into a breaking signature change. |
| CRL/OCSP revocation checking | type (existing) | roadmap | both | ARCHITECTURE.md Non-goals: "revocation" | **yes** | L | Needs a new `TrustPolicy` variant (or field) to carry revocation config. `TrustPolicy` is public, derives `Clone`/`Debug`/`Default`, and is **not** `#[non_exhaustive]` — adding a variant breaks any downstream exhaustive `match`, so this is a stop-and-ask per parity-loop's breaking-change rule, not an auto-implement. OCSP also implies network I/O this crate has never needed before. |
| kTLS offload | fn | roadmap | linux | ARCHITECTURE.md Non-goals: "kTLS offload" | no | L | Kernel TLS record offload via `SO_TLS`/`ktls` socket options — Linux-only (no macOS/Windows equivalent), and substantially larger/more invasive than everything else on this list (raw socket options, kernel-version/capability detection). Doesn't fit the "one function" sizing norm this loop otherwise holds to — recommend filing `needs-human` rather than auto-implementing sight unseen. |

## Recommended handling

- 4 gaps (async server adapter, mTLS client-side, mTLS server-side, ALPN,
  session resumption — 5 actually) are additive, normally sized, and fit
  this loop's unattended-merge criteria: implement, PR, merge on green CI.
- **Revocation** is flagged breaking — per the loop's rules this pauses for
  an explicit decision before any code is written, not a silent skip.
- **kTLS offload** is flagged oversized/out of the loop's normal shape —
  recommend labeling `needs-human` so it's filed (visible, not lost) but not
  auto-attempted, unless you'd rather it go through the normal loop anyway.
