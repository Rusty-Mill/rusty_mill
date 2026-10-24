---
category: Added
changelog: "`rusty_pk`: Ed25519 signing (`ed25519::SigningKey`, RFC 8032), constant-time, byte-identical to `ring`."
---
## 2026-10-24 - rusty_pk: Ed25519 signing (gap 5, ADR-0006)

- **Added:** `ed25519::SigningKey`, with `from_seed`, `generate`, `public_key` and `sign`, per RFC 8032 section 5.1.5–5.1.6.
  - **Fixed-base multiplication:** a new constant-time scalar multiplication on edwards25519. It runs double-and-add-always over the existing complete formulas with a masked select, 256 fixed iterations. The variable-time `mul2_vartime` stays for verification only.
  - **Scalar reductions:** the reductions modulo `L` and `S = r + k*s` use only the Montgomery core's branch-free routines.
  - **Secret material:** the expanded scalar and nonce prefix are wiped on drop, and so are the intermediates. `Debug` prints nothing.
- **Evidence:**
  - **RFC 8032 section 7.1:** tests 1 to 3 pass.
  - **Agreement with `ring`:** public keys and signatures are byte-identical for 500 deterministic seeds and message lengths 0 to 300, plus six edge seeds. Every signature verifies under our verifier and `ring`'s.
  - **Taint:** valgrind mode `ed25519-sign` marks the seed secret before key expansion and reports 0. The planted-branch control `ed25519-control` is caught.
  - **Jump counts:** exact disassembly jump counts are pinned for `mul_base`, `add`, `double`, `encode`, `reduce_digest`, `sign` and `from_seed`.
  - **Timing:** new tests `ed25519_sign_key_classes_not_distinguishable`, with an A/A baseline, have been added to the evidence collector. Three local runs gave |t| ≤ 0.28 for the key classes and ≤ 0.97 for A/A, against a threshold of 4.5.
- **Not changed:** verification, `rusty_tls`, any consumer. Wiring signing into the engine is a later PR (ADR-0006), after independent review.
