# Architecture

## Overview
`rusty_tls` gives the rusty ecosystem one TLS implementation and one trust
policy, client and server. It wraps rustls 0.23 (`ClientConnection`/
`ServerConnection`, both already sans-IO) behind a seam that hides every
rustls type from callers: a consumer names only `TlsStream`/`TlsAcceptor`/
`TlsServerStream`, `TrustPolicy`, and `Error`. Day one, the engine behind
the seam is rustls; the seam is what makes that replaceable later without
touching consumer code.

**Not goals (yet):** kTLS offload, or any hand-rolled cryptography — see
[Non-goals](#non-goals). (CRL-based revocation checking is supported —
OCSP is not, see [Boundaries](#boundaries).)

## Boundaries

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Trust decision (`TrustPolicy` → `rustls::ClientConfig`) | `trust::build_client_config`/`build_client_config_with_identity`/`build_client_config_with_alpn` | The one place a `rustls::RootCertStore`/verifier gets constructed, via a shared `client_config_builder` (the server-verification decision, independent of client identity or ALPN). `System` reads OS anchors via `platform::security::TrustAnchors` (rusty_tls#24 — the ecosystem's OS-personality crate, replacing `rustls-native-certs`; see rustils' `docs/design-discussion-tls.md`); `PinnedAnchors` takes caller-supplied DER; `DangerNoVerification` installs `danger::NoServerCertVerification`; `PinnedAnchorsWithRevocation` additionally checks presented certificates against caller-supplied CRLs, via `rustls::client::WebPkiServerVerifier::builder(..).with_crls(..)` in place of the plain `with_root_certificates` path the other verifying variants use. `TrustPolicy` is `#[non_exhaustive]` specifically so this and future variants don't repeat the breaking change adding this one was. Shared by both adapters below — this is the real reusable "core." |
| Sync transport (`TlsStream<S: Read + Write>`) | `client::TlsStream` (wraps `rustls::Stream` internally) | Never dials — accepts an already-connected `S`, so protocols that run a plaintext exchange before upgrading (RDP's X.224 negotiation) can hand over a used stream. `complete_handshake()`/`peer_certificate_der()` expose just enough handshake-derived state (raw DER, never a parsed rustls type) for a consumer like RDP's CredSSP exchange that needs the peer's certificate for its own channel binding. `new_with_client_identity()` presents a client certificate (mTLS) to a server that requests one; `new_with_alpn()`/`negotiated_alpn_protocol()` offer and read back ALPN protocols; `resumed_session()` reports whether a connection resumed a previous one (only meaningful via `TlsConnector`, below). |
| Async transport (`AsyncTlsStream<S: AsyncRead + AsyncWrite>`, feature `rusty-tokio`) | `async_client::AsyncTlsStream` | Drives the same sans-IO `rustls::ClientConnection` (`wants_read`/`wants_write`/`read_tls`/`write_tls`/`process_new_packets`) over `rusty_tokio`'s poll-based `AsyncRead`/`AsyncWrite`, via a small internal `PollAdapter` that turns `Poll::Pending` into `io::ErrorKind::WouldBlock` for rustls' synchronous `read_tls`/`write_tls` to see. `rusty_tokio` itself stays TLS-free; the dependency is optional and off by default. |
| Reusable client config (`TlsConnector`) | `connector::TlsConnector` | Builds a `ClientConfig` once (via `trust::build_client_config`) and reuses it (`Arc`-backed) across every `connect()`/`connect_async()` call — the client-side mirror of `TlsAcceptor`, and the only way session resumption actually triggers: `TlsStream::new`/`AsyncTlsStream::new` each build a fresh config (and thus a fresh, empty resumption cache) per call, so rustls' own default resumption support never gets reused state to resume *from* through them alone. |
| Server config (`TlsAcceptor`) | `server::TlsAcceptor` | The server-side mirror of the trust-decision row: builds a `rustls::ServerConfig` from a certificate chain + private key (DER, any of PKCS#8/PKCS#1/SEC1, auto-detected). Built once, cheap to reuse (`Arc`-backed) across every accepted connection — which is also what makes session resumption work on this side already: the one thing `finish_config` adds is a real `rustls::crypto::ring::Ticketer` (rustls' own `ServerConfig` default is `NeverProducesTickets`, silently disabling TLS 1.3 resumption specifically). `new_with_client_auth()` additionally requires and verifies a client certificate (mTLS) against caller-supplied client-CA roots, via `rustls::server::WebPkiClientVerifier`. `new_with_alpn()` offers ALPN protocols, read back per connection via `negotiated_alpn_protocol()` on either server stream type. |
| Sync server transport (`TlsServerStream<S: Read + Write>`) | `server::TlsServerStream` (wraps `rustls::Stream` internally) | The server counterpart to `TlsStream` — built via `TlsAcceptor::accept`, same lazy-handshake/`complete_handshake()` shape. |
| Async server transport (`AsyncTlsServerStream<S: AsyncRead + AsyncWrite>`, feature `rusty-tokio`) | `async_server::AsyncTlsServerStream` | The server counterpart to `AsyncTlsStream` — built via `TlsAcceptor::accept_async`, driving the same sans-IO `ServerConnection` over `rusty_tokio`'s poll-based I/O via its own `PollAdapter` (not shared with the client adapter's, matching this crate's existing client/server duplication convention below). Built without a confirmed live consumer (see Non-goals' consumer-gating discipline; this one was an explicit, requested exception). |

## Structure
Single crate, modular by concern rather than a workspace — there is exactly
one artifact to ship and no team/language boundary to split across:

- `error` — the one `Error` type every fallible call returns.
- `trust` — `TrustPolicy` and the only function that turns a policy into a
  `rustls::ClientConfig`.
- `danger` — the `DangerNoVerification` verifier, isolated in its own module
  so it's never an accidental `use` away from the rest of the crate.
- `client` — `TlsStream`, the sync client adapter.
- `async_client` (feature `rusty-tokio`) — `AsyncTlsStream`, the async
  client adapter.
- `connector` — `TlsConnector`, the reusable-config client-side mirror of
  `TlsAcceptor` (needed for session resumption to actually trigger).
- `server` — `TlsAcceptor` (config) and `TlsServerStream` (the sync server
  adapter it produces).
- `handrolled` (feature `handrolled-engine` **and** `--cfg
  rusty_tls_handrolled`) — the alternative engine behind the seam, reachable
  by neither by default and by neither accidentally. Nothing else in this
  crate depends on it; no type listed in [Boundaries](#boundaries) routes
  through it. Its one stage so far is `handrolled::record`, a TLS 1.3 record
  layer (RFC 8446 §5) tested three ways: byte-identical differential output
  against rustls' own `MessageEncrypter`, RFC 8448 known-answer vectors (the
  only oracle independent of rustls), and a rejection suite. Stage 2a adds
  `handrolled::der` (a strict DER reader) and `handrolled::x509` (certificate
  parsing) — **parsing only**: they report what a certificate says and decide
  nothing about whether to believe it. Stage 2b-i adds `handrolled::verify`,
  the first piece that decides anything: whether a certificate was signed by a
  given key. Authorship, never authority — it builds no chain, reads no clock,
  and enforces no constraint, so it cannot make a trust decision on its own
  and its docs say so. Stage 2b-ii adds `handrolled::path` — chain building
  and RFC 5280 §6.1 validation to a trust anchor. Stage 2b-iii adds
  `handrolled::name` — server name matching (no Common Name fallback, ever)
  and name-constraint enforcement — plus `path::verify_peer_certificate`, the
  combined entry point that does both and so cannot be half-performed. With
  that, path validation is complete. Stage 3a adds `handrolled::schedule`,
  the TLS 1.3 key schedule — where every key the protocol uses comes from, and
  where they are bound to the handshake transcript that produced them. Stage 3b
  adds `handrolled::wire` (TLS's presentation language) and
  `handrolled::handshake` (the messages themselves plus the running transcript
  hash), where parsing and encoding are required to be inverses on the wire
  bytes — the transcript covers what arrived, so a re-encoding that differs
  computes a hash the peer does not share. Stage 3c-i adds `handrolled::kx`
  (ephemeral X25519 and NIST-curve key exchange) and the TLS `SignatureScheme`
  namespace in `handrolled::verify` — including RSASSA-PSS, which the X.509
  side refuses and which RFC 8446 §4.4.3 *requires* for an RSA handshake
  signature. The two namespaces disagree about more than that, most sharply on
  where an ECDSA key's curve comes from, which is why they are separate types.
  The state machine that drives all of it (3c-ii) is not built. See
  [Non-goals](#non-goals) and ADR-0002.

There is no separate public sans-IO "core" type distinct from the two
adapters — `rustls::ClientConnection` already *is* the sans-IO engine, and
`trust::build_client_config` is the real shared logic (config
construction, independent of transport). Each adapter drives the same
`ClientConnection` methods (`wants_read`/`wants_write`/`read_tls`/
`write_tls`/`process_new_packets`) itself: `client::TlsStream` via
`rustls::Stream` over a blocking `Read + Write`; `async_client::AsyncTlsStream`
via its own poll loop (`poll_complete_io`) over `rusty_tokio`'s
`AsyncRead + AsyncWrite`. The two loops are similar in shape but distinct
in kind (blocking vs. `Poll`/`Waker`-driven) — sharing config construction
was the real duplication to remove; sharing the drive loop itself would
have meant forcing one adapter's I/O model onto the other.
`server::TlsServerStream` is the same shape as `client::TlsStream` again,
over `rustls::ServerConnection` instead — enough structural overlap to
notice, not enough (different rustls connection type, no shared trait
worth introducing for two call sites) to be worth abstracting over.

## Data flow
1. Caller connects `S` (e.g. `std::net::TcpStream`) and, for protocols that
   need it, runs any pre-TLS exchange on the raw socket.
2. `TlsStream::new(sock, server_name, policy)` builds a `ClientConfig` from
   `policy` and constructs a `rustls::ClientConnection` for `server_name`.
   No I/O happens yet.
3. The first `Read`/`Write` call drives the handshake (via
   `rustls::Stream::complete_prior_io`): plaintext in, TLS records out to
   `S`, and vice versa. Every subsequent call transparently encrypts/decrypts
   application data.
4. On a verification failure (wrong hostname, expired cert, chain that
   doesn't lead to a trusted anchor), the driving `Read`/`Write` call
   returns `Err` — there is no separate `handshake()` step to skip.

Server side mirrors this: `TlsAcceptor::new(cert_chain_der, key_der)` builds
the `rustls::ServerConfig` once (step 2's counterpart, done up front instead
of per-connection); `TlsAcceptor::accept(sock)` constructs the
`ServerConnection` for one already-accepted socket (step 2 done per
connection); the first `Read`/`Write` on the resulting `TlsServerStream`
drives the handshake exactly as step 3 describes.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
- **OCSP, kTLS offload.** Out of scope for the MVP; add only if a named
  consumer needs one, the same consumer-gate discipline rustils applies to
  its own primitives. (CRL-based revocation checking is *not* on this
  list — see `TrustPolicy::PinnedAnchorsWithRevocation` in
  [Boundaries](#boundaries). OCSP specifically would mean this crate
  either making network calls for the first time or accepting
  caller-supplied staples — a decision distinct from "check a
  caller-supplied CRL," deliberately not bundled into that variant.)
- **Any hand-rolled cryptography, ever, as the default.** rustls stays the
  engine. If a future differential-testing experiment wants to explore an
  alternative backend behind the same seam, that happens explicitly,
  never silently promoted to default.

  That experiment now exists — the `handrolled` module (rusty_tls#25), whose
  first stage is a TLS 1.3 record layer — and this non-goal is unchanged by
  it. [ADR-0002](./docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md)
  turns "explicitly, never silently" from an intention into a mechanism:
  reaching that module requires both the `handrolled-engine` cargo feature
  *and* `--cfg rusty_tls_handrolled`, because cargo features are unified
  across a dependency graph (any crate in a consumer's tree could enable one
  for everybody) and a `--cfg` flag is not. Cryptographic *primitives* remain
  out of scope entirely at every stage: the engine hand-rolls protocol, and
  AES-GCM/ChaCha20-Poly1305/X25519/SHA-2 stay `ring`.
