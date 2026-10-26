---
category: Fixed
changelog: "TLS signing design: pin hedged ECDSA to CFRG draft -05, correct nonce noise and padding, and fail closed on RNG errors."
---
## 2026-10-24 - Correct the native TLS signing nonce design

- **Fixed:** `TLS-NATIVE-SIGNING.md` pins hedged ECDSA to CFRG draft -05 §4, including 32-byte P-256 and 48-byte P-384 noise and padded HMAC inputs. Production RNG errors fail closed; plain RFC 6979 and fixed-noise paths are test-only. Fault/replay claims are bounded by §5's exclusions.
- **Changed:** ADR-0006 and the crypto plan agree on the construction and error policy, record the settled owner confirmation, and preserve no RSA signing and recorded independent review before TLS wiring.
- **Known limitations:** documentation only; no cryptographic implementation, TLS wiring, gate, default, dependency or consumer changes. This follows up merged PR #981 and its technical review comment 6385185740.
