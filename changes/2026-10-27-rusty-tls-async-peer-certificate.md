---
category: Added
changelog: `rusty_tls`'s `AsyncTlsServerStream` and `AsyncTlsStream` expose `peer_certificate_der`, as their sync counterparts do.
---
## 2026-10-27 - rusty_tls: peer_certificate_der on the async streams

- **Added:** `AsyncTlsServerStream::peer_certificate_der` returns the client's end-entity certificate after a client-auth handshake, and `AsyncTlsStream::peer_certificate_der` returns the server's. Both match `TlsServerStream` and `TlsStream`: the raw DER bytes of the first certificate the peer presented, or `None`.
- **Tests:** the async case in `tests/server_client_auth_alpn.rs` now checks the certificate each side saw, not only that the handshake completed.
- **Not changed:** `rusty_tonic` handlers still cannot see a client certificate; this adds the accessor it would need.
