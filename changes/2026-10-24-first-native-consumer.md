---
category: Changed
changelog: "ADR-0002 Amendment 2: `rleval-app`'s OIDC transport, the named Tier 1 consumer, may use `rusty_tls`'s native backend (nothing wired)."
---
## 2026-10-24 - First native consumer: rleval-app OIDC (ADR-0002 Amendment 2)

- **Decision:** the owner chose `rleval-app`'s OIDC transport as the first consumer of the native backend. It is the consumer already named under Amendment 1 for Tier 1: client only, TLS 1.3 only, named hosts. This records the working-session answer; it is not yet restated from the owner's GitHub account.
- **Amendment 2** keeps every Amendment 1 condition and adds three before wiring, which the owner accepted ("Accept all three"):
  - Tier 1 item 1's live runs are counted in the native CI job;
  - Tier 1 item 4's review also covers `handrolled/crypto/native.rs`;
  - the consumer's off-by-default feature enables `native-crypto`, the builder also passes `--cfg rusty_tls_native_crypto` (no feature or dependency can), and the one-line way back goes to rustls.
- **Docs:** ADR-0005, `TLS-NATIVE-CRYPTO-SEAM.md`, `TLS-NATIVE-SIGNING.md` and `TLS-EVIDENCE-BAR.md` note the approval. `TLS-REVIEW-SCOPE.md` adds `handrolled/crypto/native.rs` (628 lines) to the review scope.
- **Not changed:** nothing is wired. Tier 1 items 1, 3 and 4 are still unmet. No default moves. No server consumer is named.
