---
category: Changed
changelog: "rusty_aead: require the default-off aes-gcm feature for direct AES-GCM use; native TLS explicitly opts in."
---
## 2026-10-23 - Explicit AES-GCM API opt-in

- **Changed:** `rusty_aead` gates `Aes128Gcm`, `Aes256Gcm`, `GCM_NONCE_LEN`, `GCM_TAG_LEN` and their AES/GHASH implementation behind a default-off `aes-gcm` feature. Direct AES consumers must enable it; ChaCha20-Poly1305 and Poly1305 remain available by default.
- **Preserved:** `rusty_tls/native-crypto` explicitly enables `rusty_aead/aes-gcm`, retaining its suites and existing cfg gates. AES tests, examples and evidence tooling opt in explicitly.
- **Decision:** implements the owner's later 2026-10-23 concurrence, superseding #942's unconditional-export wording. The AES implementation and prior review are complete; this changes API exposure only.
- **Known limitations:** Cargo features unify across dependencies, so this opt-in is not a security boundary. No algorithm, provider selection or evidence threshold changes.
- **Validation:** `scripts/check_features.py` checks isolated consumers with defaults on/off, explicit AES opt-in and native TLS feature unification. Run AEAD tests both with and without `--features aes-gcm`.
