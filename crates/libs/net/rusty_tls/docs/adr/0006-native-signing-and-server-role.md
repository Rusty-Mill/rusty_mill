# ADR-0006: Native signing and the server role for the hand-rolled engine

Status: **Accepted** — 2026-10-24. It extends ADR-0005 from the client role to signing, the server role, session tickets
and client certificates on the first-party backend. ADR-0005 forecloses exactly this ("using this ADR to justify
native signing, a native server role ... needs its own decision"), and this is that decision.
Date: 2026-10-24
Accepted: 2026-10-24

*Provenance.* The owner asked in the working session on 2026-10-24 for gap 5 to be reopened, so that the native
engine can sign. That is the implementer's transcription, not a statement from the owner's GitHub account. The
owner then answered "Approved" in the working session on 2026-10-24 (the implementer's transcription, not yet restated from the owner's GitHub account) to the recommendations in `docs/research/TLS-NATIVE-SIGNING.md` §6
(S2 to S5). That accepted this ADR and the choices below.

Direct owner confirmation on 2026-10-24 is recorded in [PR #981 comment 6385229428](https://github.com/Rusty-Mill/rusty_mill/pull/981#issuecomment-6385229428). The S1–S5 scope is settled; the correction below specifies its nonce construction and failure policy.

## Context

ADR-0005 put the hand-rolled engine's client path on first-party primitives (`rusty_sha2`, `rusty_pk`,
`rusty_aead`, `rusty_rand`) behind `native-crypto` plus `--cfg rusty_tls_native_crypto`. Its decision 2 compiled
signing out of that build, and with it the server role, session tickets and client certificates, because no
first-party signing existed (plan gap 5, closed).

The server path needs only one thing the first-party crates lack: producing ECDSA and Ed25519 signatures from a
PKCS#8 private key. Ticket sealing needs AES-256-GCM and randomness, which both exist. Signing is secret-handling
code in the strictest sense. Reusing an ECDSA nonce across distinct messages, or leaking nonce bits across
signatures, can expose the long-term private key.

## Decision

1. **First-party signing in `rusty_pk`.**
   - ECDSA on P-256/SHA-256 and P-384/SHA-384, with a **hedged RFC 6979 nonce** pinned to
     [draft-irtf-cfrg-det-sigs-with-noise-05 §4](https://www.ietf.org/archive/id/draft-irtf-cfrg-det-sigs-with-noise-05.html#section-4).
     Follow the exact padded HMAC inputs specified in `docs/research/TLS-NATIVE-SIGNING.md` §3, with secret
     fresh `rusty_rand` noise of 32 bytes for P-256 and 48 bytes for P-384. Production RNG errors fail closed
     with no signature and no deterministic fallback. Fixed-noise and plain RFC 6979 paths are test-only.
     Repeated/zero noise retains deterministic-style security, not identical plain RFC 6979 output. Hedging
     mitigates some attacks exploiting determinism; it guarantees neither nonce uniqueness nor replay or
     fault resistance, and instruction-skipping faults are not necessarily stopped (draft §5).
   - Ed25519 per RFC 8032.
   - PKCS#8 parsing for those key types.
   - Constant-time scalar multiplication, inversion and reduction throughout. Key material is wiped on drop.
2. **No RSA signing.** On the native backend, a server certificate or client certificate has an ECDSA or Ed25519
   key. An RSA key is refused at construction with a named error. Verification of RSA signatures from peers is
   unchanged.
3. **The server role, session tickets and client certificates come back under the native backend.** The gate is
   unchanged from ADR-0005 decision 1. The ticket cipher moves behind `handrolled::crypto`.
4. **Evidence bar:** `docs/research/TLS-NATIVE-SIGNING.md` §4. That means:
   - RFC 6979 and RFC 8032 known answers;
   - a differential against `ring`;
   - nonce tests;
   - the `crypto-constant-time` job extended to every new secret path;
   - the ring-only suites re-enabled on native, except named RSA-signing tests;
   - a native BoGo floor that includes server scenarios;
   - an **independent review of the signing code, recorded with reviewer, scope and commit, before the
     `rusty_tls` wiring merges.**
5. **Unchanged:** no default moves, `rustls` stays the crate's engine, and no consumer changes. Choosing a first
   native consumer is a separate decision.

## Consequences

- **Benefit:** the native engine becomes a complete TLS endpoint without `ring` (ECDSA or Ed25519 certificates
  only), which a first native consumer will need if it serves TLS.
- **Cost:** about 9 to 13 days, plus review wait. More secret-handling code to keep under the constant-time job.
  A native server can't present an RSA certificate.
- **Foreclosed:** RSA signing on the native backend, which needs its own decision.

## Alternatives

- **Keep gap 5 closed:** the native engine stays client-only and server-side users stay on `ring`.
- **Plain RFC 6979 nonces:** deterministic and fully testable; hedging aims to mitigate some attacks
  exploiting determinism, subject to the exclusions above.
- **Random-only nonces, as `ring` does:** they depend entirely on the random source never failing.
- **Build RSA-PSS signing too:** RSA private-key operations are the hardest to make constant-time, and the plan
  has recommended never doing it since stage 7.
