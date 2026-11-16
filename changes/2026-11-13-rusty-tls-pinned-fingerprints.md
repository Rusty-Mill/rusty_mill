---
category: Added
changelog: rusty_tls can trust a server by its certificate's SHA-256 fingerprint (TrustPolicy::PinnedFingerprints).
---
## 2026-11-13 - rusty_tls fingerprint pin

- **Added:** `TrustPolicy::PinnedFingerprints(Vec<[u8; 32]>)` and `rusty_tls::fingerprint_sha256(der)`. The server is trusted by the SHA-256 of its own certificate (no chain, expiry or hostname check, so a self-signed certificate works); the handshake signature is still verified against that certificate. An empty list is `Error::NoTrustAnchors`. Requested for the mux TLS listener (rusty_term D236; Nano, 2026-11-13).
- **Known limitations:** not supported by the opt-in native engine (it returns `Unsupported`, as for the other policies it lacks). No new dependency; `TrustPolicy` is `#[non_exhaustive]`, so the variant is additive.
