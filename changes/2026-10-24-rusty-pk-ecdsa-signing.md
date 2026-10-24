---
category: Added
changelog: "`rusty_pk`: ECDSA signing on P-256/SHA-256 and P-384/SHA-384 with pinned draft-05 hedged nonces."
---
## 2026-10-24 - rusty_pk: ECDSA signing (gap 5, ADR-0006)

- **Added:** `ecdsa::SigningKey::{from_bytes, curve, public_key, sign}`, returning DER. No TLS wiring or RSA signing.
- **Nonce:** pinned `draft-irtf-cfrg-det-sigs-with-noise-05` section 4, reconciled with design correction #989 at `263832fa92552fe6f0a7d99f168bba5a9344054c`. Noise precedes the secret key; P-256 uses 32 bytes and zero padding 63/32, P-384 uses 48 bytes and padding 31/80. Both d/f use the same noise, with V updated between them; other RFC 6979 steps and rejection sampling remain unchanged. `hedge_len(curve)` replaces the unreleased fixed `HEDGE_LEN` constant.
- **RNG/fault behavior:** detected RNG failure returns `SignError::Random` without a signature or fallback, including partial writes. Undetected repeated/zero noise gives a deterministic instance of the padded construction, not plain RFC 6979 output. Repeated noise retains deterministic-style resistance to algebraic recovery caused by weak randomness in randomized ECDSA. Hedging mitigates some attacks exploiting determinism; it guarantees neither uniqueness nor replay protection and need not stop instruction-skipping faults. Other side-channel defenses remain necessary.
- **Known answers:** separate test-only original RFC 6979 appendix A.2.5/A.2.6 vectors; 20 fixed-noise draft-05 fixtures from stdlib Python HMAC/hashlib and separate affine big-integer curve arithmetic. K/V, nonce and r/s are pinned for both curves, zero/nonzero noise and SHA padding boundaries. The production signing path matches these fixtures and ring verifies them. The reference is independently implemented, not independently human-reviewed; draft -05 has no completed vectors.
- **Bounded validation:** both-curve cross-verification with ring (200 keys per curve), 2,000 repeated-message signatures per curve with deterministic test noise, fixed-zero reproducibility, both-curve fail-closed RNG errors and invalid-key/DER tests. A second, separately written Python reference agrees with all 20 fixtures.
- **Evidence on this construction:** constant-time claims remain "no leak found by the methods run", not proof.
  - No repeated `r` over 100,000 signatures of one message per curve with OS CSPRNG noise. This is an ignored test, run once in release locally; see the PR.
  - A nonce top-bit smoke test: over 20,000 DRBG nonces per curve, each of the top 8 bits is within 5 sigma of uniform. Statistical only.
  - Valgrind taint (`ecdsa-sign256`/`384`: 4 verdict sites, the rest through `ecdsa::output`; control refused) and the exact jump-count pins re-run on the draft-05 code.
  - P-256 and P-384 timing with A/A baselines in the collector. Each key pair gives both classes equal `r` and `s` encoded lengths, which the test asserts. Local figures are in the PR.
- **Outstanding before TLS wiring:** independent review with reviewer, scope and exact commit. The full evidence plan (engine suites, native BoGo with server scenarios, PKCS#8) remains incomplete.
- **Design documents:** adopt #989's exact nonce/RNG/fault corrections in the note, ADR and replacement plan, retaining direct owner scope and independent review before TLS wiring.
