---
category: Added
changelog: "`rusty_tls`: on the native backend, the server role, session tickets and client certificates with ECDSA and Ed25519 keys (no RSA signing)."
---
## 2026-10-24 - rusty_tls: native signing and the server role (gap 5, ADR-0006)

- **Added (native backend only):** under `native-crypto` with `--cfg rusty_tls_native_crypto`, the hand-rolled engine has the server role, TLS 1.2 server, session tickets, the server half of version negotiation, and client certificates.
  - `SigningKey::ecdsa_p256`, `ecdsa_p384` and `ed25519` load PKCS#8 through `rusty_pk::pkcs8`, which follows `ring`'s acceptance rules, and sign with `rusty_pk`.
  - ECDSA nonces are hedged with noise from the engine's random source.
- **No RSA signing on native (ADR-0006 decision 2):** `SigningKey::rsa` returns the new `SignError::RsaUnsupported`. Verifying a peer's RSA signatures is unchanged.
- **Both backends:**
  - The ticket cipher (AES-256-GCM) and the server's randomness now go through `handrolled::crypto` instead of calling `ring` directly. Same algorithms; the `ring` build is unchanged in behaviour.
  - `SignError` gains `RsaUnsupported`. It is `non_exhaustive`.
- **Tests and CI:**
  - The 11 suites marked `ring-only suite (ADR-0005)` run on native. The RSA signing cases are skipped there by name, and two expectations follow native's ChaCha20-first preference.
  - The native CI job runs OpenSSL interop in both directions, and requires 36 or more suites.
  - Native BoGo includes server scenarios: 494 pass, 0 fail, with a floor of 480. The BoGo shim no longer gates anything on the native cfg.
- **Docs:** README, BoGo README and the `TLS-NATIVE-SIGNING.md` checkpoint.
- **Not changed:** no default, no consumer, and `rustls` stays the crate's engine. The owner's S5 review (ADR-0006, as amended by #1001) approved `7a3267f5a` on 2026-10-25; it is recorded in plan section 11b.
