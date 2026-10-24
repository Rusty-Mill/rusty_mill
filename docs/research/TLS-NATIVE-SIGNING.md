# Native signing and the server role: scope for gap 5

Date: 2026-10-24. Status: **scope, no code; S1 to S5 decided and ADR-0006 accepted on 2026-10-24.** The owner asked on 2026-10-24 for gap 5 to be reopened so the
native engine can sign. This is the implementer's transcription of a working-session instruction, not yet restated
from the owner's GitHub account. The decisions in section 6 are the owner's: the owner answered "Approved" in the working session on 2026-10-24 (the implementer's transcription, not yet restated from the owner's GitHub account) to the recommendations. The governing record is
ADR-0006 (`crates/libs/net/rusty_tls/docs/adr/0006-native-signing-and-server-role.md`), accepted. No gate,
default or consumer changes until the implementation PRs land.

Context: `CRYPTO-REPLACEMENT-PLAN.md` section 13.1 gap 5 ("ECDSA P-256/P-384 (RFC 6979 or hedged nonce), Ed25519
sign, PKCS#8 parse. RSA-PSS signing (stage 7) stays 'recommend not doing'"). Gap 4 (`TLS-NATIVE-CRYPTO-SEAM.md`)
put the client role on first-party primitives and compiled signing, the servers, tickets and client certificates
out of the native build (ADR-0005 D3). This note scopes putting them back without `ring`.

## 1. Answer first

- **What is missing is signing.** Everything else the server role needs already has a first-party implementation:
  the record layers, key schedules, key exchange, verification, randomness, and AES-256-GCM for tickets.
- **The new secret-handling code is small but sharp:**
  - ECDSA P-256 and P-384 signing, with a nonce that must never repeat or leak a bit;
  - Ed25519 signing, which needs a constant-time fixed-base scalar multiplication that `rusty_pk` does not have yet;
  - parsing PKCS#8 private keys.
- **RSA signing stays out**, as the plan has recommended since stage 7. So a native server presents only ECDSA or
  Ed25519 certificates, and a native client certificate is ECDSA or Ed25519 too.
- **Effort:** about 9 to 13 focused days, excluding review wait. Signing primitives and their evidence take about
  6 to 8 (the plan's figure). Rewiring the server role, tickets and client certificates on the native backend, plus
  re-enabling the suites, takes about 3 to 5.
- **No default moves.** The native backend stays opt-in, and `rustls` stays the crate's engine (ADR-0002).

## 2. What `ring` still does on the server path

Counted on `main` at `b123ab74` (2026-10-24):

| `ring` use | File | Native replacement | Status |
| --- | --- | --- | --- |
| ECDSA P-256 / P-384 signing (`EcdsaKeyPair::from_pkcs8`, `sign`) | `sign.rs` | `rusty_pk::ecdsa` signing (new) | **Not implemented** |
| Ed25519 signing (`Ed25519KeyPair::from_pkcs8_maybe_unchecked`) | `sign.rs` | `rusty_pk::ed25519` signing (new) | **Not implemented** |
| RSA-PSS signing | `sign.rs` | none | **Out of scope** (decision S3) |
| `SystemRandom` for ECDSA nonces | `sign.rs` | `rusty_rand::fill`, used inside a hedged nonce (decision S2) | Implemented (`rusty_rand`) |
| AES-256-GCM ticket sealing, and a random nonce per ticket | `ticket.rs` | `rusty_aead::Aes256Gcm`, `rusty_rand::fill` | Implemented (`aes-gcm` feature) |

Everything else in `server.rs`, `server12.rs` and the server half of `negotiate.rs` already goes through
`handrolled::crypto` (#865, #871). Under the native cfg they are compiled out only because they need a `SigningKey`.

## 3. Design

1. **Signing primitives in `rusty_pk`** (reusing what exists):
   - **ECDSA (`ecdsa::SigningKey`).**
     - `k·G` reuses the constant-time double-and-add-always scalar multiplication behind `ecdh` (complete Renes,
       Costello and Batina formulas, masked select, fixed iteration count).
     - `k⁻¹ mod n` uses Fermat inversion. The exponent is public and the base is not branched on.
     - `s = k⁻¹(z + r·d) mod n` uses the order field's constant-time arithmetic.
     - `r = 0` and `s = 0` are retried with a fresh nonce, never returned.
     - Signatures are DER-encoded, as TLS needs.
   - **Nonce: hedged RFC 6979** (decision S2). The RFC 6979 HMAC-DRBG is seeded with the key, the message hash and
     32 fresh bytes from `rusty_rand`, following `draft-irtf-cfrg-det-sigs-with-noise`. A broken random source then
     degrades to deterministic RFC 6979, which is still safe. A fault or replay cannot force a repeated nonce on a
     good one. A test-only entry point with no added randomness reproduces the RFC 6979 appendix A.2.5 and A.2.6
     vectors exactly.
   - **Ed25519 (`ed25519::SigningKey`).**
     - SHA-512 key expansion comes from `rusty_sha2`, with clamping.
     - A new constant-time fixed-base multiplication over edwards25519 uses complete twisted-Edwards addition,
       double-and-add-always and a masked select. The existing `mul2_vartime` stays for verification only.
     - The 64-byte hash is reduced mod L in constant time. That is a wide reduction through the Montgomery core,
       not a branchy one.
     - The output is the RFC 8032 signature, deterministic by definition.
   - **PKCS#8 parsing.** RFC 5208 / 5958 PrivateKeyInfo for:
     - `id-ecPublicKey` with an RFC 5915 ECPrivateKey on P-256 or P-384;
     - `id-Ed25519` (RFC 8410), both the v1 and v2 forms.

     It uses the existing strict DER reader. It rejects a scalar that is zero or not below `n`, and a public key
     that does not match the private key.
   - **Key material** lives in wipe-on-drop types, like `ecdh::PrivateKey`.
2. **`rusty_tls`.**
   - **Native `SigningKey`.** The uninhabited native `SigningKey` becomes an enum over `rusty_pk`'s ECDSA and
     Ed25519 keys. `SigningKey::rsa` does not exist on native: an RSA key is refused at construction, with a named
     error.
   - **Server role and tickets.** `server`, `server12`, `ticket`, `negotiate`'s server half and `ClientIdentity`
     come back under the native cfg. `ticket.rs` moves its AES-256-GCM and random fill behind `handrolled::crypto`,
     like the record layers.
   - **Signature schemes.** The server's offered scheme list on native omits the RSA-PSS schemes, because it cannot
     sign them. Verification is unchanged: a native endpoint still verifies RSA signatures from its peer.
3. **No new gate.** The ADR-0005 construction stays as it is: `native-crypto` plus `--cfg rusty_tls_native_crypto`
   plus `--cfg rusty_tls_handrolled`.

## 4. Evidence (plan sections 11 and 13.1, gap 5)

| Check | How |
| --- | --- |
| Known answers | RFC 6979 A.2.5 and A.2.6 (P-256 and P-384, with SHA-256 and SHA-384), through the test-only deterministic path. RFC 8032 section 7.1 Ed25519 vectors. Wycheproof `ecdsa_secp{256,384}r1_*` and `ed25519_test.json`: our own signatures must verify, and the verify vectors keep passing. |
| Differential vs `ring` | Ed25519 signatures byte-identical to `ring` for the same key and message. Both ECDSA backends verify each other's signatures, because `ring`'s nonces are random. The same PKCS#8 inputs give the same public keys, and both backends accept or reject the same malformed keys. |
| Nonce | The deterministic path matches RFC 6979 exactly. The hedged path never repeats `r` over 100,000 signatures of one message under one key. A forced-zero random source still signs correctly. A nonce-bias check of the top bits of `k` over many signatures shows no deviation from uniform, a statistical smoke test only. |
| Constant time | The `crypto-constant-time` job gains ECDSA signing (scalar multiplication, inversion, the `s` computation, nonce derivation) and Ed25519 signing (fixed-base multiplication, scalar reduction): valgrind taint with the private key and nonce marked secret, exact disassembly jump counts, and the timing series against an A/A baseline with a planted-leak control. The claim stays "no leak found by these methods". |
| Engine | The 11 suites marked `ring-only suite (ADR-0005)` run on the native backend again, except the tests that need an RSA signing key, which are gated individually and named. The native BoGo floor is re-set from a run that includes server scenarios. OpenSSL interop runs in the server direction. |
| Review | An independent review of the new signing code before it is wired (plan section 13.2 D4). Its reviewer, scope and conclusion are recorded in plan section 11b. Automated reviews are labelled automated. |

## 5. Effort

About 9 to 13 focused days, excluding review wait:

| Work | Days |
| --- | --- |
| ECDSA signing with the hedged nonce | 3 to 4 |
| Ed25519 signing with fixed-base multiplication and scalar reduction | 2 to 3 |
| PKCS#8 parsing | 1 |
| Constant-time evidence | 1 to 2 |
| `rusty_tls` server role, tickets and suites | 2 to 3 |

Rough, plus or minus 50 percent, like the plan's figures. Suggested order of PRs:

1. ADR-0006 and this note.
2. `rusty_pk` Ed25519 signing.
3. `rusty_pk` ECDSA signing.
4. PKCS#8 parsing.
5. Constant-time evidence.
6. Review.
7. `rusty_tls` wiring.

## 6. Decisions for the owner

| # | Decision | Options | Recommendation |
| --- | --- | --- | --- |
| S1 | Reopen gap 5 | Reopen for native ECDSA and Ed25519 signing (server role, tickets, client certificates), or keep it closed | **Asked for by the owner on 2026-10-24** (working session; this note records it) |
| S2 | ECDSA nonce | Plain RFC 6979 (deterministic); hedged RFC 6979 (RFC 6979 plus fresh randomness); or random only, as `ring` does | **Decided 2026-10-24: hedged RFC 6979** |
| S3 | RSA signing | Keep it out (native servers and client certificates use ECDSA or Ed25519 only), or build RSA-PSS signing (plan stage 7, "recommend never") | **Decided 2026-10-24: kept out** |
| S4 | ADR | Accept ADR-0006 (extends ADR-0005 to signing and the server role), or amend ADR-0005 | **Decided 2026-10-24: ADR-0006 accepted** |
| S5 | Review before wiring | Independent human review of the signing code before the `rusty_tls` wiring merges, or the owner alone (plan D4) | **Decided 2026-10-24: independent review, recorded with reviewer, scope and commit** |

## 7. Not in scope

- RSA signing (decision S3) and any change to verification.
- Any consumer, and any default. A first native consumer is a separate decision, next in the owner's queue.
- `ring` elsewhere in the lockfile (#979), AES-GCM evidence gaps (#978).

## 8. Limits

Counts and file lists come from `grep` on `main`. Nothing here was compiled or prototyped. Effort figures are
estimates. Reuse claims (the constant-time Weierstrass multiplication, Fermat inversion, the DER reader, SHA-512)
are from reading `rusty_pk` and `rusty_sha2` at that commit.
