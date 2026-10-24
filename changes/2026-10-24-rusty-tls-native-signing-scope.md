---
category: Changed
changelog: "Crypto plan: scope native signing and the server role for the hand-rolled engine (gap 5); ADR-0006 accepted."
---
## 2026-10-24 - crypto plan: scope for native signing and the server role (gap 5)

- **Added:**
  - `docs/research/TLS-NATIVE-SIGNING.md`: what `ring` still does on the server path, the design (ECDSA P-256/P-384 with a hedged RFC 6979 nonce, Ed25519, PKCS#8, no RSA signing), the evidence bar, the effort (about 9 to 13 days) and decisions S1 to S5.
  - `crates/libs/net/rusty_tls/docs/adr/0006-native-signing-and-server-role.md`, **accepted** on 2026-10-24 (decisions S2 to S5 approved in the working session).
- **Changed:** the plan's §0, the gap 5 row in §13.1 and the D2 entry record gap 5's reopening and decisions S2 to S5, as the implementer's transcription.
- **Not changed:** any code, gate, default, dependency or consumer.
