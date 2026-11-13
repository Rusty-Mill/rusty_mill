---
category: Added
changelog: rusty_tls: `TrustPolicy::PinnedCertSha256` trusts a server by its pinned SHA-256 certificate fingerprint, with `certificate_sha256` to compute it.
---
## 2026-11-13 - rusty_tls trusts a pinned certificate fingerprint

- **Added:** `TrustPolicy::PinnedCertSha256(Vec<[u8; 32]>)`, which trusts a server only if the SHA-256 fingerprint of its end-entity certificate is one of the pins, and `rusty_tls::certificate_sha256` to compute one (for example from `peer_certificate_der()` on first use). This is the trust hook the rusty_mux TLS listener needs for its user-supplied, client-pinned certificate (D236, TLS-LISTENER.md step 3), so its client no longer has to use `DangerNoVerification`.
- **Security:** the handshake signature is still verified against the pinned certificate's key, so a server that only copied the public certificate is refused. The chain, validity period and server name are not checked; the caller's per-host pin stands in for them.
- **No new dependency:** SHA-256 comes from the ring provider rustls already uses; `Cargo.toml` and the crate's rust-version (1.87) are unchanged.
- **Known limitations:** the native (hand-rolled) engine does not support this policy; it refuses it as `Unsupported`, as it does every policy other than `System` and `PinnedAnchors`.
