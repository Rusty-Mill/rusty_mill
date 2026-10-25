---
category: Added
changelog: "`rusty_pk`: PKCS#8 private-key parsing for ECDSA P-256/P-384 and Ed25519 keys, matching `ring`'s acceptance rules."
---
## 2026-10-24 - rusty_pk: PKCS#8 private keys (gap 5, ADR-0006)

- **Added:** `pkcs8::ecdsa(curve, der)` and `pkcs8::ed25519(der)`, returning the existing `ecdsa::SigningKey` and `ed25519::SigningKey`. `pkcs8::KeyRejected` carries no key material.
  - **ECDSA:** a v1 `PrivateKeyInfo` with exactly `id-ecPublicKey` and the requested curve. Inside it, an RFC 5915 `ECPrivateKey` of version 1 with a scalar of the curve's length in `[1, n-1]`. Optional `[0]` parameters must name the same curve. The `[1]` public key is required and must equal `d * G`. These are `ring`'s rules.
  - **Ed25519:** v1 with no public key, or v2 with a public key. The v2 key may be a primitive `[1]` or the legacy constructed `[1]`, and it must match the seed.
  - **Common:** `[0]` attributes are ignored. Any other trailing data, and any non-minimal DER, is rejected.
- **Secrets:** parsing branches only on structure. Secret bytes go straight to the constant-time constructors. The derived public key is compared with the encoded one in fixed work, and that verdict goes through `ecdh::ensure`.
- **DER reader:** gains `peek`, `small_version` and `bit_string`, with unit tests.
- **Evidence:**
  - **Differential against `ring`:** 20 generated keys per type, hand-built edge cases, and every single-byte mutation (masks 0x01, 0x80 and 0xff), truncation and one-byte extension of each base document. Each case checks that both accept or both reject, and that both derive the same public key when they accept. The edge cases are `[0]` present or absent or naming another curve, a missing or wrong public key, a short, zero or equal-to-n scalar, the wrong curve, both v2 public-key encodings, and the RFC 8410 v1 key.
  - **Taint:** new modes `pkcs8-ecdsa256` and `pkcs8-ed25519` mark only the private-key bytes secret. Every report is an `ensure` verdict: exactly 3 for ECDSA, 1 for Ed25519.
- **Disassembly pin:** `ecdh::generator_x` goes from 9 to 16 jumps. Its calls are unchanged, but LLVM now inlines the `Curve` drop glue. The extra jumps are capacity checks and cleanup on public sizes, confirmed by comparing against main's disassembly.
- **Docs:** the `TLS-NATIVE-SIGNING.md` checkpoint records PKCS#8 and keeps the owner's S5 review and the engine work outstanding.
- **Not changed:** `rusty_tls`, any consumer.
