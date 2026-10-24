---
category: Added
changelog: "`rusty_pk`: ECDSA signing on P-256 (SHA-256) and P-384 (SHA-384) with hedged RFC 6979 nonces (draft-irtf-cfrg-det-sigs-with-noise-05 section 4), constant-time."
---
## 2026-10-24 - rusty_pk: ECDSA signing (gap 5, ADR-0006)

- **Added:** `ecdsa::SigningKey`, with `from_bytes`, `curve`, `public_key` and `sign(message, fill)`, plus `ecdsa::noise_len`. It signs P-256 with SHA-256 and P-384 with SHA-384, as TLS pairs them, and returns DER.
  - **Nonce:** hedged RFC 6979, pinned to `draft-irtf-cfrg-det-sigs-with-noise-05` section 4 (the construction #989 sets). RFC 6979 section 3.2's HMAC-DRBG, with steps (d) and (f) replaced by `K = HMAC_K(V || tag || Z || pad1 || int2octets(x) || pad2 || bits2octets(h1))`. `Z` is `noise_len` fresh secret bytes (32 on P-256, 48 on P-384), the same in both steps, and the zero pads end each part on a hash-block boundary.
  - **Random-source errors fail closed:** a `fill` that reports an error makes `sign` return `SignError::Random` and sign nothing. There is no deterministic fallback.
  - **Claims, bounded by the draft's section 5:** noise that is weak or repeated without being detected still leaves the nonce a deterministic function of key, message and noise. That avoids the key recovery that weak randomness causes in random-nonce ECDSA, but the output is not plain RFC 6979. Hedging mitigates some side-channel and fault attacks on determinism. It does not guarantee nonce uniqueness, prevent replay, or necessarily stop instruction-skipping faults. Plain RFC 6979 and fixed-noise paths are test-only.
  - **Constant-time arithmetic:** `k*G` reuses ECDH's constant-time multiplication, through the new crate-internal `ecdh::generator_x`. `k^-1` and `s = k^-1(z + r d)` use the order field's branch-free arithmetic. Every secret-derived check goes through `ecdh::ensure`: key range, nonce range, not at infinity, and r and s non-zero.
  - **Public outputs:** DER encoding of the published `r` and `s` happens only in `ecdsa::output`.
- **Evidence:**
  - **Known answers:**
    - RFC 6979 appendix A.2.5 and A.2.6 ("sample" and "test" on both curves) match exactly through the test-only plain path.
    - Fixed-noise answers (all-zero, all-0xff and a 1..n ramp `Z`, both curves) match through the public `sign`. The expected values come from an independent Python implementation of the draft-05 construction, which also reproduces A.2.5 and A.2.6.
  - **Cross-check with `ring`:** for 200 keys per curve, each side verifies the other's signatures, and `ring` accepts our public key.
  - **Nonces:**
    - No repeated `r` over 2,000 signatures of one message per curve (every run). There is also an ignored test for the plan's bar of 100,000 per curve with OS noise; it was run once locally in release (see the PR).
    - A top-bit bias smoke test: over 20,000 nonces per curve, each of the top 8 bits of `k` is within 5 sigma of uniform. This is statistical only.
    - Fixed noise repeats, different noise changes the nonce, and zero noise does not equal plain RFC 6979.
    - A failed source returns `SignError::Random`, and invalid keys are refused.
  - **Taint:** valgrind modes `ecdsa-sign256` and `ecdsa-sign384` mark the key and the noise secret. Exactly 4 reports are verdict branches; every other report's stack passes through `ecdsa::output`. `verdict_sites.py --output`, with new self-tests, enforces this. The planted control `ecdsa-control` is caught and refused.
  - **Jump counts:** exact counts are pinned for `sign_seeded`, `Rfc6979::mac` and `generator_x`.
  - **Timing:** new P-256 and P-384 sparse-vs-dense key tests, each with an A/A baseline, are in the collector. Each pair is chosen so that both classes' `r` and `s` have the same encoded lengths, which the test asserts; an early pair with unequal DER lengths showed a harmless public-length gap.
- **Not changed:** verification, `rusty_tls`, any consumer. `TLS-NATIVE-SIGNING.md` is left to #989.
