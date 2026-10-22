# ADR-0005: First-party cryptographic primitives behind the hand-rolled client engine

Status: **Accepted** — 2026-10-22, client role only. Supersedes ADR-0002 §6 for the
primitives the hand-rolled engine's client path uses, and for nothing else.
Date: 2026-10-08 (drafted as `docs/research/CRYPTO-REPLACEMENT-PLAN.md` §10)
Accepted: 2026-10-22

*Provenance.* The owner approved this in the working session on 2026-10-22, answering the
recommendations for decisions D2, D3 and D4 in `docs/research/TLS-NATIVE-CRYPTO-SEAM.md`
with "Approved". This is the implementer's transcription of that answer, not a statement
from the owner's GitHub account.

## Context

The hand-rolled engine (ADR-0002) takes every primitive from `ring`. ADR-0002 §6 kept it
that way and said revisiting it "is its own issue, with its own ADR". This is that ADR.

`docs/research/CRYPTO-REPLACEMENT-PLAN.md` built first-party replacements in
`crates/foundation`: `rusty_sha2` (SHA-2, HMAC, HKDF), `rusty_pk` (signature verification,
X25519, ECDH on P-256 and P-384), `rusty_aead` (ChaCha20-Poly1305, AES-GCM) and `rusty_rand`
(OS randomness). Gap 4 of that plan, the `rusty_tls` backend seam, was reopened for the
client role on 2026-10-22. Its first step (#865) put every client-path primitive behind one
internal module, `handrolled::crypto`, still backed by `ring`.

The primitives differ in what can be verified. Hashes, HMAC, HKDF and signature
verification handle only public data, so vectors, differential tests and fuzzing are the
right evidence. Everything that handles a secret key can leak through timing or cache
effects that none of those techniques can observe.

`ring` is also used by `rustls` and by `boringtun`, `quinn`, `jsonwebtoken` and others in the
lockfile. Replacing it under the hand-rolled engine does not remove it from the build.

## Decision

1. **The hand-rolled engine's client path may use the first-party crates instead of `ring`.**
   The backend is chosen by the ADR-0002 §1 construction a second time: a cargo feature
   `native-crypto`, which adds the first-party crates, and the build flag
   `--cfg rusty_tls_native_crypto`, which no dependency can set. Both only take effect
   inside the existing `handrolled-engine` feature and `rusty_tls_handrolled` flag. Without
   them the engine uses `ring`, as today.
2. **Under the native backend, signing is compiled out (D3).** `sign.rs`, the TLS 1.3 and
   1.2 servers, session tickets and `ClientIdentity` do not exist in that build, so `ring`
   is not in its dependency graph. The native engine is a client that presents no
   certificate. A server asking for one gets an empty Certificate, as a client with no
   identity configured does today.
3. **Under the native backend, ChaCha20-Poly1305 is offered first (D4).** Portable AES-GCM
   is about 60 to 150 times slower than `ring`'s, so the client prefers
   `TLS_CHACHA20_POLY1305_SHA256` and still offers the AES-GCM suites. The cipher-suite
   order, and so the ClientHello's fingerprint, differs from the `ring` build. The `ring`
   build's order does not change.
4. **The evidence bar is the one in `TLS-NATIVE-CRYPTO-SEAM.md` §4.** A CI variant of the
   engine job runs the client-side suites under the native backend, with its own list of
   supported tests, zero-tests guard and BoGo floor. A seam differential runs the same keys
   and inputs through both backends. The `crypto-constant-time` job and the plan's §11
   table apply. The `ring` job keeps full coverage. Constant-time claims stay "no leak found
   by these methods on this toolchain", never proof.
5. **Unchanged:** RSA signing is not built (plan stage 7). `ring` stays a dev-dependency
   oracle for the first-party crates. Nothing here becomes a default, and no consumer is
   wired. ADR-0002 §3 (never the default) and Amendment 1 (`rleval-app` stays on `ring`
   unless separately approved) still hold.

## Alternatives considered

- **Keep `ring` for everything.** This was the status quo and ADR-0002 §6. It is still the
  default build. It was rejected as the only option because the first-party crates exist,
  and the client path needs none of the dangerous stages.
- **Replace only the public-data primitives (hashes, HMAC, verification) and keep `ring`
  for the rest.** That is the plan's recommended floor. It would not take `ring` out of the
  engine's graph, which is the point of gap 4.
- **Keep `ring` only for signing** (D3's other option). The primitives would be native but
  the build would not be ring-free. It was rejected because the client role needs no
  signing.
- **Keep AES-GCM first with portable AES** (D4's other option), or use AES-NI intrinsics.
  The intrinsics approval from 2026-10-08 was never reconfirmed (plan D1). Putting a slow
  cipher first without a measured reason was rejected.

## Consequences

- **Accepted:** auditability and a ring-free client engine, at the cost of:
  - slower portable AES-GCM;
  - a maintenance burden;
  - weaker evidence than `ring` for secret-handling code;
  - a second ClientHello fingerprint.

  A native build cannot act as a server or present a client certificate.
- **Still required before the native backend merges:** the review status of AES-GCM
  (`TLS-NATIVE-CRYPTO-SEAM.md` D5, open). The plan's standing rule is that each
  secret-handling stage is reviewed before it is wired, and the owner reviews alone (plan
  D4, 2026-10-10).
- **Created:** the native backend in `handrolled::crypto`, the `native-crypto` feature and
  cfg, the client-only CI variant, and the seam differential.
- **Foreclosed:** using this ADR to justify native signing, a native server role, or any
  default change. Each of those needs its own decision.
