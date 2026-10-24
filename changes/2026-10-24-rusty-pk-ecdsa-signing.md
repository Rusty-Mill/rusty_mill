---
category: Added
changelog: "`rusty_pk`: ECDSA signing on P-256 (SHA-256) and P-384 (SHA-384) with hedged RFC 6979 nonces, constant-time."
---
## 2026-10-24 - rusty_pk: ECDSA signing (gap 5, ADR-0006)

- **Added:** `ecdsa::SigningKey`, with `from_bytes`, `curve`, `public_key` and `sign(message, fill)`. It signs P-256 with SHA-256 and P-384 with SHA-384, as TLS pairs them, and returns DER.
  - **Nonce:** hedged RFC 6979. The section 3.2 HMAC-DRBG runs over the key and the message hash, with 32 fresh bytes from the caller's source as section 3.6's "additional data". A dead source degrades to deterministic RFC 6979, and a failing one signs nothing.
  - **Constant-time arithmetic:** `k*G` reuses ECDH's constant-time multiplication, through the new crate-internal `ecdh::generator_x`. `k^-1` and `s = k^-1(z + r d)` use the order field's branch-free arithmetic. Every secret-derived check goes through `ecdh::ensure`: key range, nonce range, not at infinity, and r and s non-zero.
  - **Public outputs:** DER encoding of the published `r` and `s` happens only in `ecdsa::output`.
- **Evidence:**
  - **Known answers:** RFC 6979 appendix A.2.5 and A.2.6 ("sample" and "test" on both curves) match exactly through the deterministic path.
  - **Cross-check with `ring`:** for 200 keys per curve, each side verifies the other's signatures, and `ring` accepts our public key.
  - **Nonces:** no repeated `r` across 2,000 hedged signatures per curve. A zero hedge is deterministic and valid. A failed source returns `SignError::Random`. Invalid keys are refused.
  - **Taint:** valgrind modes `ecdsa-sign256` and `ecdsa-sign384` mark the key and the hedge secret. Exactly 4 reports are verdict branches; every other report's stack passes through `ecdsa::output`. `verdict_sites.py --output`, with new self-tests, enforces this. The planted control `ecdsa-control` is caught and refused.
  - **Jump counts:** exact counts are pinned for `sign_with_extra`, `Rfc6979::mac` and `generator_x`.
  - **Timing:** new tests `ecdsa_p256_sign_sparse_vs_dense_key`, with an A/A baseline, are in the collector. The two key classes give signatures of equal DER length, because a first pair whose signatures differed in length showed a harmless public-length gap. Six local runs gave |t| 0.07–1.01, against an A/A range of 0.26–2.29 and a threshold of 4.5.
- **Docs:** `TLS-NATIVE-SIGNING.md` now names the nonce construction actually used, RFC 6979 section 3.6.
- **Not changed:** verification, `rusty_tls`, any consumer.
