---
category: Added
changelog: "`rusty_tls`: `TlsAcceptor::new_with_client_auth_and_alpn`, so a server can require a client certificate and negotiate ALPN (for example `h2`) on one connection."
---
## 2026-10-21 - rusty_tls: a server that requires a client certificate and negotiates ALPN

- **Why:** the gRPC transport (card t45, phase 4b, `rusty_hyper`) needs a server behind mutual TLS that also negotiates `h2`. `TlsAcceptor` had `new_with_client_auth` and `new_with_alpn` as separate constructors, so a server could do only one of the two. This is the server-side counterpart of the client constructors from #813.
- **Added:** `TlsAcceptor::new_with_client_auth_and_alpn` (the arguments of `new_with_client_auth` plus `alpn_protocols`). One crate-private builder behind it and `new_with_client_auth`; an empty list offers no protocol, as before.
- **Tests:** 8 in `tests/server_client_auth_alpn.rs` (7 sync, 1 async through `accept_async`), written first (all failed on `todo!()`): the server sees `h2` and the client certificate together; a client offering no protocol connects with none negotiated; a client offering only `http/1.1` is refused; no client certificate, or one from an untrusted CA, is still refused when the protocol matches; an empty protocol list negotiates nothing but still requires a certificate; empty client CA roots and a bad key are errors. Dropping the ALPN list, or the client verifier, from the builder fails tests (3 and 5 of 8).
- **Version:** `rusty_tls` 0.11.0 -> 0.12.0 (additive public constructor; `crates/libs/net/rusty_tls/docs/versioning.md` §2). Release notes and tag are left to the release cut (§4).
- **Not changed:** no behavior of an existing constructor. `AsyncTlsServerStream` still has no `peer_certificate_der` (the sync stream does); not added here.
