---
category: Added
changelog: "`rusty_tls`: `new_with_client_identity_and_alpn` on `TlsStream` and `AsyncTlsStream`, so a client can present a certificate and offer ALPN (for example `h2`) on one connection."
---
## 2026-10-21 - rusty_tls: a client identity and ALPN together

- **Why:** the gRPC transport (card t45, phase 4b) needs mutual TLS and `h2` negotiation on the same connection, and `rusty_a2a`'s gRPC binding already tests mTLS. `new_with_client_identity` and `new_with_alpn` were separate constructors, so a client could do only one of the two.
- **Added:** `TlsStream::new_with_client_identity_and_alpn` and `AsyncTlsStream::new_with_client_identity_and_alpn` (same arguments as `new_with_client_identity`, plus `alpn_protocols`). One crate-private config builder behind both.
- **Tests:** 8 in `tests/client_identity_alpn.rs` and `tests/async_client_identity_alpn.rs`, written first (all failed on `todo!()`): against a rustls server that requires a client certificate and offers `h2`, the server sees the ALPN offer and the certificate, `negotiated_alpn_protocol` reads `h2` after the handshake, a server with no ALPN leaves it unnegotiated, a certificate from the wrong CA is refused, and an invalid key or server name is an error. Dropping the ALPN offer, or the identity, from the new builder fails a test.
- **Validation:** the client ALPN offer is checked before it reaches rustls (shared `validate_alpn`): an empty protocol name (a rustls debug assertion), a name over 255 bytes (length truncated on the wire) and an offer over 32 KiB are the new `Error::InvalidAlpnProtocols`, for the existing `new_with_alpn` as well as the new constructors. `tests/alpn_validation.rs` has the boundaries; the empty-name and oversize cases panicked inside rustls before.
- **Version:** `rusty_tls` 0.10.1 -> 0.11.0 (additive public constructors and an `Error` variant; `crates/libs/net/rusty_tls/docs/versioning.md` §2). Release notes and tag are left to the release cut (§4).
- **Not changed:** the server side (`TlsAcceptor` still does not verify client certificates, tracked separately), `TlsConnector`, and the existing constructors.
