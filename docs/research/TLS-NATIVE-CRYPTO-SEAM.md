# Native engine off `ring`: scope for gap 4 (the `rusty_tls` backend seam)

Date: 2026-10-21. Status: **step 1 merged (#865); step 2, the native backend, merged as #871 (2026-10-23). Gap 4 reopened for the
client role (D1) and D2 to D4 decided on 2026-10-22: ADR-0005 accepted, signing compiled out, ChaCha20-Poly1305
first; the owner confirmed D2 to D4 on GitHub (#868 comment 6343682194).** D5's review question was settled on
2026-10-23 by the owner's attestation that all reviews were complete (plan section 11b). Its export question
was decided the same day: the AES types stay exported unconditionally. D5 is closed. The section 4 re-check of the plan's section 11 table is plan
section 11b. Every decision in section 6 is the owner's.

Context: `CRYPTO-REPLACEMENT-PLAN.md` section 13.1, gap 4 ("same signatures as the `ring` call sites,
selected by an extra cfg beside `rusty_tls_handrolled`; default stays `ring`"). This note turns that row
into a concrete design, an evidence plan and the decisions that block it.

## 1. Answer first

- Every primitive the hand-rolled engine takes from `ring` has a first-party replacement **except
  signing** (gap 5, closed). So an engine that calls no `ring` is the **client role with no client
  certificate**, which is where the plan recommends stopping (plan section 1).
- The work is one internal module plus mechanical call-site changes: about **6 to 9 days** (the plan's 5
  to 8, plus about 1 for compiling signing out and the CI variant).
- It was blocked by decisions, not code. D1 to D4 were decided on 2026-10-22 (ADR-0005); AES-GCM's
  review status (D5) is still open and must be settled before the native backend merges (section 6).
- This removes `ring` from the *engine*. rustls, the default engine, keeps it, and so do `boringtun`,
  `quinn`, `jsonwebtoken` and the other lockfile users (plan decision D5).

## 2. Call-site inventory

`ring` appears in 10 files of `crates/libs/net/rusty_tls/src/handrolled/` (counted 2026-10-21 on `main`
at `6ba272c8`):

| `ring` use | Files | Replacement | Status |
| --- | --- | --- | --- |
| `digest` SHA-256 / SHA-384 | `schedule.rs` | `rusty_sha2` | Implemented |
| `hmac` (key schedules, Finished; HKDF is hand-written over it) | `schedule.rs`, `schedule12.rs` | `rusty_sha2::Hmac` (or its HKDF) | Implemented |
| `aead` ChaCha20-Poly1305, records | `record.rs`, `record12.rs` | `rusty_aead::ChaCha20Poly1305` | Implemented |
| `aead` AES-128-GCM / AES-256-GCM, records | `record.rs`, `record12.rs` | `rusty_aead::{Aes128Gcm, Aes256Gcm}` | Implemented, portable only. Automated, bounded source review on #651, no human review (plan section 11b). Decision D5 |
| `agreement` X25519, P-256, P-384 | `kx.rs` | `rusty_pk::x25519`, `rusty_pk::ecdh` | Implemented |
| `signature` verification (the 11 verifiers listed in plan section 0) | `verify.rs` | `rusty_pk::{rsa, ecdsa, ed25519}` | Implemented, exactly that subset |
| `rand::SystemRandom` | `client.rs`, `kx.rs`, `server.rs`, `ticket.rs` | `rusty_rand::fill` | Implemented |
| `aead` AES-256-GCM, session tickets | `ticket.rs` (used only by `server.rs`) | `rusty_aead::Aes256Gcm` | Implemented; server-side only |
| `signature` signing (ECDSA, Ed25519, RSA; PKCS#8) | `sign.rs` | none | **Not implemented (gap 5, closed)** |

## 3. Design

1. **One internal backend module, `handrolled/crypto.rs`.** It holds small types (hash, HMAC, a sealer
   and opener per cipher suite, key exchange, verify, random fill) with a `ring` implementation and a
   native one. Every call site goes through it, so the backend changes in one file, not ten. It replaces
   `ring`'s helper types (`Nonce`, `Aad`, `UnboundKey`, `LessSafeKey`, `hmac::Context`) with the typed
   arrays the native crates use.
2. **Selection uses the ADR-0002 section 1 construction, a second time:** a cargo feature
   `native-crypto` that adds the native crates instead of `ring`, and a cfg
   `--cfg rusty_tls_native_crypto` that a dependency cannot set, together with the existing
   `--cfg rusty_tls_handrolled`. `native-crypto` does not require `handrolled-engine`, so a
   native build has no direct `ring` dependency (the narrow ADR-0002 §1 exception in ADR-0005,
   owner decision 2026-10-22). Without the native cfg the engine uses `ring`, as today.
   `rustls`'s own `ring` is untouched.
3. **Under the native cfg, signing is compiled out:** `sign.rs`, `server.rs`, `server12.rs`, `ticket.rs`
   and `ClientIdentity`. Keeping them would keep `ring` in the graph (decision D3). A client asked for a
   certificate answers with an empty one, as it does today with no identity configured.
4. **Errors.** `ring::error::Unspecified` maps to the existing error types. A `rusty_rand` failure is
   propagated, never replaced with zeroed bytes (plan section 13.1, gap 1).
5. **No new secret-dependent code in the module.** Tag checks stay inside the native crates'
   constant-time comparisons, and key material lives in the native types, which wipe on drop.

## 4. Evidence (plan sections 11 and 13.1, gap 4)

| Check | How |
| --- | --- |
| The client-side engine suite under the native backend | A second variant of the `rusty-tls-engine` CI job (`.github/workflows/ci.yml`) with the native cfg. Its client tests against a real `rustls` server then double as interop with a `ring`-backed peer. Server and mTLS suites cannot run there, because section 3 compiles them out, so the variant needs its own list of the tests it supports and its own zero-tests guard. It is **not** equivalent coverage: the existing `ring`-backend job stays unchanged and keeps running everything. |
| OpenSSL interop and BoGo | OpenSSL interop in the client direction (`handrolled_client12_socket_interop`) runs in the variant. **BoGo does not yet:** its shim drives the engine's server and does not build on the native backend, so a client-only shim is follow-up work; until then BoGo covers the `ring` backend only, at its existing floor (830). The owner agreed to this deferral on 2026-10-22 (ADR-0005 decision 4). It is a deferral, not equivalent coverage. |
| Seam differential | Same keys and inputs through both backends: byte-identical records, secrets and Finished values. RFC 8448 known answers under both. |
| Constant time | The existing `crypto-constant-time` job covers the primitives; the seam adds no secret-dependent branch or index, checked in review. |
| Plan section 11 table | Re-checked for #871 in plan section 11b (2026-10-23). The re-check lists the review records for each primitive the backend calls. Still without a recorded human review: the #540 secret-handling code at `e3266f2e` (HMAC key paths, ChaCha20-Poly1305, X25519, the Montgomery-core invariants), the `constant_time_eq` and `wipe` helpers, the seam code in #871 (automated reviews only), and AES-GCM (an automated, bounded source review only). On 2026-10-23 the owner attested that all these reviews were complete (plan section 11b). The commit, method and findings are not recorded. |

## 5. Effort

About 6 to 9 focused days, excluding review wait: the module and call sites (about 4 to 5), compiling
signing out under the cfg (about 1), the CI variant and seam differential (about 1 to 2), docs (about
1). Rough, plus or minus 50 percent, like the plan's own figures.

## 6. Decisions for the owner

| # | Decision | Options | Recommendation |
| --- | --- | --- | --- |
| D1 | Reopen gap 4 | Reopen for the client role, or keep it closed | **Decided 2026-10-22: reopened, client role only** |
| D2 | ADR | ADR-0002 section 6 requires its own ADR to revisit `ring`. Accept the plan's section 10 draft (ADR-0005), scoped to the client role, or amend ADR-0002 section 6 | **Decided 2026-10-22: ADR-0005 accepted, client role only** |
| D3 | Signing under the native cfg | Compile out the server and client certificates (ring-free), or keep `ring` for signing only (native primitives, not ring-free) | **Decided 2026-10-22: compile out** |
| D4 | Cipher-suite order with portable AES (60 to 150 times slower than `ring`) | Put ChaCha20-Poly1305 first under the native cfg (a ClientHello fingerprint change), or confirm the AES intrinsics approval (plan D1, never reconfirmed) | **Decided 2026-10-22: ChaCha20-Poly1305 first** |
| D5 | AES-GCM review | Whether the 2026-10-13 review of #651 is gap 3's review; and whether `rusty_aead`'s AES types stay exported unconditionally (`AES-GCM-DESIGN.md` section 10). An automated, bounded source review exists. Its re-review of `70139bb3` (comment 6150642627) cleared the source corrections. It explicitly did not accept a constant-time evidence record or production adoption, and no human review is recorded (plan section 0, "AES-GCM review record") | **Review question settled 2026-10-23** by the owner's attestation (plan section 11b). **Exports decided 2026-10-23:** they stay unconditional, because their only user is the doubly gated native backend. **Closed.** |

## 7. Not in scope

- `rustls` and every consumer's default path. The default engine keeps `ring`.
- Signing (gap 5) and RSA signing (plan stage 7, "recommend never").
- `ring` elsewhere in the lockfile (plan D5).
- Any consumer. `rleval-app`'s Tier 1 opt-in (ADR-0002 Amendment 1) stays on the `ring` backend unless
  the owner approves the native backend for it with its own evidence.

## 8. Limits

Counts and file lists come from `grep` on `main` at `6ba272c8`; I did not compile or prototype the
module. Effort figures are estimates. Native crate status is as recorded in `CRYPTO-REPLACEMENT-PLAN.md`
and `AES-GCM-DESIGN.md`, not re-verified here.
