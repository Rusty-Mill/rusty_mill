# Native replacement for `ring`: assessment and staged plan

Date: 2026-10-08. Scope: the primitives `rusty_tls`'s native engine takes from `ring`.
Status: **assessment and plan only. No primitive was written, no crate created, no
consumer touched.** Everything marked "decision" is the owner's. Section 10 is a draft ADR
and is **not accepted**.

## 0. Progress and owner decisions (updated as stages land)

Owner decision 2026-10-08: **`unsafe` intrinsics approved; stop after stage 4.** Stage 5
(AES-GCM) and later are therefore out of scope unless the owner reopens them; the AES-NI
approval is recorded but unused until then. Section 9 items 3 to 5 were not answered; I
took the recommended defaults and say so here: new crates, not extensions of `rusty_rsa`
(revisit at stage 2); one additive function, `rusty_crypto_key::wipe`, for zeroizing
non-heap secrets; fiat-crypto not yet evaluated (stage 4).

| Stage | State | Evidence so far |
| --- | --- | --- |
| 0 Harness | **Done** (`rusty_ct_check`) | Taint tool, timing t-test and disassembly audit each catch a planted leak and pass a clean probe; Wycheproof loader. |
| 1 SHA-2, HMAC, HKDF | **Done** (`rusty_sha2`) | 18 tests plus doctest: FIPS 180 examples, RFC 4231 case 1, six Wycheproof files (HMAC and HKDF, SHA-256/384/512), differential vs `ring` over lengths 0 to 300 and block boundaries, long inputs, HMAC key lengths 0 to 1000, 600 random HKDF calls; 9 hand-made mutants all caught; valgrind taint run clean with a leaky control that is caught; disassembly budget pinned; timing test `|t|` 1.75 (HMAC-SHA256) and 0.56 (HMAC-SHA512), one run. |
| 2 Signature verify | **Done** (`rusty_pk`: `rsa`, `ecdsa`, `ed25519`) | See "Stage 2 results" below. |
| 3 ChaCha20-Poly1305 | **Done** (`rusty_aead`) | See "Stage 3 results" below. |
| 4 X25519 | not started | |

### Stage 2 results (`rusty_pk`, 2026-10-08)

Layout taken: one crate, `rusty_pk`, with a shared Montgomery core (`mont`, branch-free
`mul/add/sub`), `field`, and modules `rsa`, `ecdsa`, `ed25519` (X25519 joins it at stage 4).
Not an extension of `rusty_rsa`: its `BigUint` stays untouched.

- **Accepts exactly what `ring` accepts**, including its quirks, read from `ring` 0.17.14
  source and then checked by differential test: RSA modulus length is rounded up to bytes
  before the 2048 floor, max 8192 bits, `e` odd in 3..2^33-1, signature length equals modulus
  length; PSS salt = hash length; ECDSA strict DER; Ed25519 accepts a non-canonical `y`
  and an `x = 0` key with the sign bit set (RFC 8032 rejects both) and requires canonical `S`.
  These choices are **looser than the RFC** by design, to match `ring`; changing them is a
  policy decision, not a bug fix.
- **Evidence:** 42 tests (plus one ignored speed test). Every case in 19 vendored Wycheproof files (RSA PKCS#1 2048/3072/4096
  x SHA-256/384/512, RSA-PSS incl. the parameter zoo, ECDSA P-256/SHA-256, P-384/SHA-384,
  P-384/SHA-256, Ed25519) gets the same verdict from `ring` and from us, and the
  vector's stated verdict. P-256/SHA-384 (no Wycheproof file; `ring` cannot sign it) uses 72
  cases from an independent Python signer. 18 RSA padding forgeries (block type, pad byte,
  separator, trailing bytes, PSS trailer, top bit, salt length) from a throwaway key and an
  independent signer. 3,000 `ring`-signed-then-mutated cases and several thousand mutated Wycheproof
  vectors give the same verdict as `ring`; 2000 garbage inputs never panic. Deterministic
  mutation fuzzing only: **no coverage-guided fuzzer was run** (none installed).
- **Mutation check by hand, more than 20 mutants:** all caught after two rounds of repair. The first
  round exposed two real test gaps (padding checks were not individually exercised; a
  non-minimal DER length was caught only by accident), both fixed. One mutant is equivalent
  and stays alive: removing the `signature == 0` check changes nothing, because a zero
  signature decodes to a zero encoded message that fails padding.
- **A wrong test assumption, not a code bug:** Wycheproof's salt-0 PSS file contains a vector
  (tcId 69, "s_len changed to 32", result invalid) that is a genuine salt-32 signature, so it
  verifies under the shared salt-equals-hash profile in both `ring` and ours.
- **Speed, measured once on this VM, release (`tests/perf.rs`, ignored):**

  | | ours | `ring` | ratio |
  | --- | --- | --- | --- |
  | RSA-2048 PKCS#1 | 76 us | 29 us | 2.7x |
  | RSA-3072 | 210 us | 56 us | 3.8x |
  | RSA-4096 | 298 us | 101 us | 2.9x |
  | ECDSA P-256 | 542 us | 73 us | 7.5x |
  | ECDSA P-384 | 1172 us | 709 us | 1.7x |
  | Ed25519 | 435 us | 48 us | 9.0x |

  RSA matches the section 5 estimate; P-256 and Ed25519 are slower than estimated (no
  windowing or precomputation yet). All are under 1.2 ms, so a handshake with three
  signatures adds about 1 to 2 ms. Optimisation is possible and not done.
- **Constant time:** not applicable to verification (public data); the `_vartime` routines are
  named and documented. The `mont` primitives that stage 4 will use on secrets are not yet
  tainted-tested.

### Stage 3 results (`rusty_aead`, 2026-10-08)

- **Evidence:** 12 tests plus doctest: RFC 8439 Poly1305 and AEAD examples, all 325 Wycheproof
  cases (nine wrong-nonce-size vectors are unrepresentable in the typed API and are asserted
  invalid), seal and open byte-identical to `ring` at every length 0 to 200 and at 255, 256,
  257, 1000, 4096 and 16384, 300 damaged-input cases with the same verdict as `ring`, a
  failed open leaves the buffer undecrypted, Poly1305 streaming at every split point, and the
  final-reduction edge cases (accumulator exactly `p - 1`, `p`, `p + 3`) with tags computed by an
  independent big-integer implementation.
- **Mutation check, 22 mutants, all caught.** The first pass left one real gap: nothing
  drove Poly1305's branch-free final reduction to the "subtract p" outcome, so replacing the
  select with a constant passed every test, including Wycheproof. Fixed with the edge-case
  vectors above. The block-counter bound (`MAX_LEN`, 256 GiB) cannot be run, so only its value
  is pinned by a unit test.
- **Constant-time evidence (`scripts/ct_check.sh`, rustc 1.99.0, x86-64):**
  - valgrind taint, key and plaintext secret: `seal` reports 0; a planted branch on secret
    data reports 1 (so the check can fail).
  - `open` reports exactly 1, in `open_in_place`: the accept/reject branch on the tag
    comparison. That bit is public by design and cannot be declassified from outside the
    library, so the script requires exactly one report located there; a second one fails.
  - disassembly: `ChaCha20::block` has 1 conditional jump (the round-loop back-edge) and the
    Poly1305 block function has 0; both pinned.
  - timing (scheduled job, two runs): fixed vs other key `|t|` 1.6 and 1.8; all-zero vs
    all-ones plaintext 0.6 and 0.5; tag mismatch in first vs last byte 2.6 and 1.5 (threshold 4.5).
  - Limits: x86-64 only; Poly1305 `update`/`finalize` and the tag assembly branch on public
    lengths and are covered by taint but not by the disassembly budget.
- **Speed (once, VM, release; `tests/perf.rs`):** 305 MB/s at 1 KiB, 341 MB/s at 16 KiB, 330 MB/s at
  16 MiB, against `ring` at 1306, 1779 and 1804: 4.3x to 5.5x slower, inside the section 5
  estimate (3 to 6x). No SIMD yet.
- **Toolchain:** the sandbox's Rust was updated from 1.97.0 to 1.99.0 mid-session at the
  owner's request. The stage 1 and 2 checks were rerun on 1.99.0 and still pass, and the
  disassembly budgets did not move; this is the compiler-drift check the plan calls for, done once.

## 1. Answer first

- **Stages 1 and 2 (hashes, HMAC, HKDF, signature verification) are worth doing.** They touch
  only public data, so the usual evidence (vectors, Wycheproof, differential against `ring`,
  fuzzing) is the right evidence. About 25 to 35 days.
- **Stage 3 (ChaCha20-Poly1305) and stage 4 (X25519) are defensible**, with a constant-time
  evidence bar I propose below. About 6 to 8 days together.
- **Stage 5 (AES-GCM) is the real cliff.** Portable constant-time AES is 50 to 100 times slower
  than `ring` (estimate, section 5). Usable for a TLS client on a WAN, wrong for bulk transfer
  on a fast link, unless the owner approves AES-NI intrinsics (`unsafe`).
- **Stage 6 (ECDH/ECDSA/Ed25519 secret-scalar maths) is feasible but is where review matters
  more than any test.** **Stage 7 (RSA sign) I recommend never doing**; an RSA server key can
  move to ECDSA or stay on `ring`.
- **Removing `ring` from the build is not reachable through `rusty_tls` alone.** Ten packages
  in `Cargo.lock` depend on it (section 2.2). The honest goal of this track is "the native TLS
  engine does not need `ring`", not "the lockfile has no `ring`".
- **A client-only engine needs no signing.** Stages 1 to 5 plus ECDH (the first half of stage
  6) are enough for `rusty_request`'s use. Signing (server role, client certificates) is what
  drags in the dangerous stages. That is a useful place to stop.
- I did not find a reason to overturn the earlier D2 recommendation for the *dangerous* stages.
  I did find that the safe stages are cheap enough to be worth doing on their own merits
  (sovereignty, auditability, no C/asm build step), so the decision is no longer all or
  nothing.

## 2. Inventory of what `ring` provides today

### 2.1 In `rusty_tls` (`src/handrolled/`; `ring` 0.17.14, optional via `handrolled-engine`)

| Primitive | `ring` API used | Parameters | Where | Secret-dependent |
| --- | --- | --- | --- | --- |
| SHA-256, SHA-384 | `digest::digest` | one-shot, transcript hashes | `schedule.rs:103-129` | No |
| HMAC-SHA-256/384 | `hmac::Key/sign/Context/verify` | keys = traffic secrets | `schedule.rs:110,141-153,410-427` | Key yes; MAC compare must be constant time (`hmac::verify` today) |
| HKDF-Extract/Expand | built on HMAC (hand-written in `schedule.rs`, not `ring::hkdf`) | TLS 1.3 labels | `schedule.rs:140-212` | Keys yes |
| AES-128-GCM, AES-256-GCM | `aead::LessSafeKey` | 12-byte nonce, 16-byte tag, record AAD | `record.rs:117-334` | Yes |
| ChaCha20-Poly1305 | same | same | `record.rs` | Yes |
| AES-256-GCM (tickets) | `aead::LessSafeKey` | random 12-byte nonce, fixed AAD | `ticket.rs:77-224` | Yes |
| X25519 | `agreement` (ephemeral) | 32-byte keys; all-zero output refused | `kx.rs:115-190` | Yes |
| ECDH P-256, P-384 | `agreement` (ephemeral) | uncompressed SEC1 points; on-curve check | `kx.rs` | Yes |
| ECDSA verify P-256/SHA-256, P-384/SHA-384 (+ P-256/SHA-384, P-384/SHA-256 for X.509) | `signature::UnparsedPublicKey`, `*_ASN1` | DER signatures, SEC1 uncompressed keys | `verify.rs:324-338, 639-681` | No |
| Ed25519 verify | `signature::ED25519` | 64-byte signatures | `verify.rs` | No |
| RSA PKCS#1 v1.5 verify SHA-256/384/512 | `RSA_PKCS1_2048_8192_*` | 2048 to 8192 bit, RSAPublicKey DER | `verify.rs:328-330` | No |
| RSA-PSS verify SHA-256/384/512 (TLS only) | `RSA_PSS_2048_8192_*` | salt length = digest length (checked in `ring` source) | `verify.rs:632-634` | No |
| ECDSA sign P-256, P-384 | `EcdsaKeyPair::from_pkcs8`, `sign` | PKCS#8 in, DER out, random nonce | `sign.rs:95-116, 191` | **Yes** |
| Ed25519 sign | `Ed25519KeyPair::from_pkcs8_maybe_unchecked` | PKCS#8 v1 and v2 | `sign.rs:128` | **Yes** |
| RSA-PSS sign SHA-256/384/512 | `RsaKeyPair::from_pkcs8`, `sign` | 2048 to 8192 bit, CRT | `sign.rs:140, 201` | **Yes** |
| Randomness | `rand::SystemRandom` | client random, ephemeral keys, ticket keys/nonces, PSS salt, ECDSA nonces | `client.rs:104,946`, `server.rs:1747`, `kx.rs`, `ticket.rs`, `sign.rs` | Yes (quality) |

Notes the earlier assessment did not have:

- HKDF is not `ring::hkdf`; it is 40 lines over `ring::hmac`. The replacement surface is
  smaller than "HKDF" suggests: SHA-2 and HMAC.
- Curves: P-256 and P-384 only (no P-521, no secp256k1); RSA verify 2048 to 8192; RSA sign
  PSS only (no PKCS#1 v1.5 signing). Certificates with PSS or P-521 are refused today (F7), so
  parity does not require them.
- `ring` hides several behaviours the replacement must match or document. Verified in
  `ring` 0.17.14 source: Ed25519 verify requires a canonical `S < L` (`Scalar::from_bytes_checked`)
  and uses a variable-time double scalar multiplication; RSA verify accepts exponents from 3
  up to 2^33 - 1 ("more flexible in verification"); RSA-PSS requires salt length equal to
  digest length; ECDSA signatures are strict DER positive integers and keys must be
  uncompressed points. Each is a place where a "correct" implementation can still diverge.
  Not verified here: ring's exact modulus checks and SHA-1 legacy handling.
- The native engine prefers AES-256-GCM, then AES-128-GCM, then ChaCha20-Poly1305
  (`client.rs:510-512`). So most real handshakes land on the slowest suite for a portable
  implementation (section 5).

### 2.2 `ring` elsewhere in the workspace

`Cargo.lock` lists these dependents of `ring`: `boringtun`, `jsonwebtoken`, `quinn-proto`,
`rcgen`, `rp-router`, `rp-server`, `rustls`, `rustls-webpki`, `rusty_tls`, `sct`.
`rustls` dependents include `reqwest`, `sqlx-core`, `tokio-postgres-rustls`, `ureq`,
`rusty_request`, `rusty_rdp`, `agentgateway-tls`. Consequences:

1. Even with a fully native `rusty_tls`, `ring` stays in the lockfile until these leave too
   (the `rustls`-based TLS track and `boringtun`, `jsonwebtoken`, `quinn` are separate
   problems).
2. Two small direct uses are not TLS and are **already replaceable today with no new crypto**:
   `rp-server` (`routes.rs:1320`) draws a token with `ring::rand`, which `rusty_rand` provides;
   `rp-router` (`lib.rs:4105`, a test) uses `ring::hmac`, which stage 1 would cover. I did
   not change them (consumers are out of scope).

## 3. Reusable code in the foundation crates

| Crate | What it has | Constant time? | Evidence | Use |
| --- | --- | --- | --- | --- |
| `rusty_rsa` `Sha256` | streaming SHA-256, 244 lines, FIPS 180-4 | **Structurally regular**: no data-dependent branch or index (round constants indexed by round number, fixed schedule). Not checked with tooling. | Read `process_block`. | Stage 1 starting point. Not 384/512. Slow (218 MB/s measured). |
| `rusty_rsa` `BigUint` | unsigned bignum, `Vec<u32>` limbs, schoolbook mul, **bit-at-a-time long division**, square-and-multiply `modpow`, extended-Euclid `mod_inverse` | **No, and says so.** `mul` skips zero limbs; `compare`, `divmod`, `modpow` branch on values; `to_bytes_be_padded` trims with `remove(0)`; limb count leaks magnitude. Doc comment forbids secret exponents. | Read in full; doc comments. | **Not reusable for secrets, and too slow even for public RSA** (below). |
| `rusty_oauth` `crypto::ecc`, `jwt::es256` | P-256 point arithmetic over `BigUint`, ES256 sign + verify, RFC 6979 | **No**: built on `BigUint`; its own header says so (Montgomery ladder and deterministic nonce are mitigations, not constant time). | Header read; code skimmed, not audited. | Useful as an *oracle* and for RFC 6979 structure; it is an existing variable-time signer on a secret key in the tree, which this track does not fix. |
| `rusty_rdp` `crypto::aes` | table AES (S-box computed at key setup) | **No**, says so | Header | Not usable (lookup tables on secrets). |
| `rusty_rdp` / `rusty_oauth` `crypto::hmac` | HMAC-SHA-256 copies | Regular | Not read in full | Duplicates; stage 1 would let both delete theirs later (consumers, not now). |
| `rusty_crypto_key` | `SecretBytes` (zeroize via `write_volatile`, heap `Vec` only), `constant_time_eq` | `constant_time_eq` folds length and uses `black_box`; best effort, not proven | Read | Reuse for heap secrets and tag compare. **It contains `unsafe`** (the volatile wipe), so a `forbid(unsafe_code)` crypto crate may depend on it but cannot reproduce it. Needs an additive `wipe(&mut [u8])` / fixed-array wrapper for stack secrets (decision). |
| `rusty_rand` | `fill`/`bytes`, `/dev/urandom` behind a global `Mutex<File>` on Unix, `BCryptGenRandom` on Windows | n/a | Read | Replaces `SystemRandom`. Caveats: on Linux `/dev/urandom` does not block before the pool is initialised (`getrandom(2)` does); chroot or fd exhaustion fail the open; global mutex serialises callers. Acceptable, but a documented difference from `ring`. |
| `rusty_sha1` | SHA-1 | n/a | | Not needed (SHA-1 is refused by the engine). |
| `rusty_simd` | f32/f16 dequantisation kernels, `unsafe` AVX2/NEON | n/a | | **Nothing for crypto.** The name suggests otherwise. |

Measured on this machine (4 vCPU Xeon @ 2.1 GHz with AES-NI, PCLMUL, AVX2, SHA-NI; `--release`;
single run, VM noise not controlled; script in the session scratchpad, not in the repo):

| Operation | Existing `rusty_rsa` | `ring` 0.17.14 | Ratio |
| --- | --- | --- | --- |
| SHA-256, 16 MiB | 218 MB/s | 1320 MB/s | 6x |
| RSA-2048, e=65537, public op | 5.88 ms | 0.026 ms (verify of a bad signature, which still does the modexp) | ~225x |
| RSA-3072 | 12.27 ms | 0.058 ms | ~210x |
| RSA-4096 | 21.57 ms | 0.113 ms | ~190x |

So even for *public* RSA, `BigUint` is two orders of magnitude off. A chain with three
RSA-2048 signatures costs about 18 ms of verification; with RSA-4096 roots about 65 ms. That
is why stage 2 needs a new fixed-width Montgomery implementation, not a reuse.

## 4. Constant-time evidence

### 4.1 What is installed here

Checked in this environment (rustc/cargo 1.97.0 stable only; Linux x86-64):

| Tool | Present | Notes |
| --- | --- | --- |
| valgrind 3.22 (memcheck) | yes | Basis for a ctgrind-style check. |
| `objdump` (binutils), gdb, clang, gcc | yes | Disassembly checks. |
| `perf`, `cargo-fuzz`, `cargo-mutants`, `cargo-llvm-cov` | **no** | `cargo-fuzz` also needs nightly, which is not installed. Fuzzing would be a stable deterministic harness, as `rusty_tls` already does. |
| `dudect` crate / tool | no | A Welch's-t harness is about 40 lines; I wrote one as a probe. |
| Jasmin, ct-verif, other formal CT tools | no | Not evaluated. |

### 4.2 Probe: do the three methods work here?

I wrote a throw-away program (scratchpad, not committed) with one leaky and one clean
function and ran each method on it. Results, all single runs:

| Method | Leaky comparison / table index | Clean comparison |
| --- | --- | --- |
| **dudect-style** (Welch's t, 200k measurements per class, slowest 5% cropped, fixed vs differing at byte 0) | `|t|` = 4097 | `|t|` = 0.4 (threshold about 4.5) |
| **valgrind, secret marked undefined** via the client-request instruction sequence (needs ~10 lines of `asm!`, so `unsafe` in a test-only harness) | branch on secret: 2 "Conditional jump depends on uninitialised value"; secret table index: 1 "Use of uninitialised value of size 8" | no report |
| **objdump** | conditional jumps countable per function, but my first attempt matched the wrong symbols (generic closures); needs a symbol-exact extraction script, which I did not finish | |

So: valgrind and the timing test both work in this sandbox and both caught a planted leak.
That is all this shows. It is a test that the *tools run*, not evidence about any real code.

### 4.3 What each method can and cannot show

None proves the absence of a leak.

| Method | Can show | Cannot show |
| --- | --- | --- |
| **Valgrind taint (ctgrind style)** | A branch, or a memory address, that depends on a value marked secret, on the paths and inputs actually executed. Deterministic, no statistics. Good per-PR gate. | Anything not executed; variable-latency instructions (`div`, early-exit multipliers on some CPUs) because they are neither branches nor addresses; leaks via data that was not marked; declassification mistakes (marking too little); behaviour of a different compiler, flags or target. |
| **dudect-style timing** | A leak large enough to measure on this machine with these input classes. Catches things the model misses (variable-latency ops, library calls). | Small leaks (remote attackers average over millions of samples); leaks that depend on input classes nobody thought of; anything under VM noise. A pass means "not detected". Must run on pinned hardware; flaky as a per-PR gate, fine as a scheduled job. |
| **Disassembly audit** (script over `objdump` for conditional jumps, `div`/`idiv`, jump tables, in functions on the secret path) | The compiler did not turn a mask-select into a branch **in this build**. Catches the classic LLVM regression. | Anything about another rustc version, opt level or target; variable-latency instructions unless listed; microarchitectural effects. Must be rerun on every toolchain bump. |
| **Review** (a person who knows the attack literature) | Logic errors in formulas and structure. | Compiler behaviour. Not replaceable by tools, and not something this session can supply. |
| **Not covered by anything above** | Speculative execution, power, EM, fault attacks, cache attacks that need a co-resident attacker with finer resolution than the dudect clock. | |

### 4.4 Method I propose (decision: the evidence bar is the owner's)

1. **By construction.** `#![forbid(unsafe_code)]`; secrets live in types with no `PartialEq`,
   `Ord`, `Index`, `Display`; masks from a tiny `ct` module (select, eq, lt, swap) built on
   `black_box`; variable-time routines carry a `_vartime` suffix and a CI grep fails if a
   secret-handling module calls one. No `/` or `%` on secret-derived values.
2. **Valgrind taint test** for every secret-consuming entry point, per PR (needs a test-only
   `unsafe` harness crate, or the owner can allow one dev-dependency; both are an owner call).
3. **Disassembly script** per PR on the pinned toolchain: no conditional jump in functions
   tagged secret except an allowlist of reviewed, public-bound loops; no `div`.
4. **dudect-style job**, scheduled (nightly), on a fixed runner, at least 10^6 measurements
   per class, `|t|` below 4.5 across the run, with the class pairs written down next to the
   code (fixed vs random key, fixed vs random message, equal vs unequal tag).
5. **Independent review** for stages 4 to 6 before any use in `rusty_tls`.
6. **Stated scope**: "no leak detected by methods 2 to 4 on rustc X, x86-64 and aarch64".
   Never "constant time".

## 5. Performance estimate against `ring`

Baselines measured here: `ring` AES-128-GCM about 9.1 GB/s, AES-256-GCM 7.6 GB/s,
ChaCha20-Poly1305 1.8 GB/s (best of five 16 MiB seals). **Everything about the native
implementations below is an estimate from my knowledge of portable implementations, not a
measurement; I wrote no primitive.** Treat each as plus or minus a factor of two.

| Stage | Portable Rust estimate | vs `ring` here | Acceptable? |
| --- | --- | --- | --- |
| 1 SHA-256/384/512, HMAC, HKDF | 250 to 450 MB/s (existing code: 218 measured) | 3 to 6x slower | Yes. TLS hashes kilobytes; the key schedule is tens of microseconds. |
| 2 RSA-2048/3072/4096 verify (Montgomery, 64-bit limbs) | 40 to 80 µs / 90 to 180 µs / 160 to 320 µs | 2 to 4x slower | Yes. A chain adds well under 1 ms. |
| 2 ECDSA P-256 / P-384 verify | 0.2 to 0.4 ms / 0.8 to 1.5 ms | 3 to 5x | Yes. |
| 2 Ed25519 verify | 0.1 to 0.2 ms | 2 to 4x | Yes. |
| 3 ChaCha20-Poly1305 | 300 to 600 MB/s | 3 to 6x | Yes for WAN-bound downloads (below 4 Gbit/s); no for 10 GbE or loopback bulk. |
| 4 X25519 | 60 to 150 µs per op | 2 to 4x | Yes. |
| 5 AES-GCM, bitsliced/fixsliced AES + constant-time GHASH | 60 to 150 MB/s | **60 to 150x** | Client over a WAN, yes. Bulk, LAN or loopback: no. With AES-NI/PCLMUL (`unsafe`) it would be near `ring`. |
| 6 ECDH P-256 / P-384 | 0.3 to 0.6 ms / 1 to 2.5 ms | 3 to 8x | Yes per handshake. |
| 6 ECDSA/Ed25519 sign | 0.2 to 2 ms | 3 to 8x | Yes for a server doing hundreds of handshakes per second, not thousands. |
| 7 RSA-2048 sign (CT modexp, CRT) | 3 to 8 ms | 3 to 6x | Server-side capacity cost; the risk (section 8) is the issue, not speed. |

Consumer check: `rusty_request` is a client; its TLS cost is dominated by bulk records, so
**stage 5 decides whether the native engine is usable for large downloads.** A mitigation
that needs no `unsafe`: when the build has no hardware AES path, the native client orders
ChaCha20-Poly1305 first (it is faster than portable AES). That changes the ClientHello
fingerprint and must be a deliberate, documented choice.

## 6. Test-vector sources (offline vs vendored)

Network from this sandbox reached `raw.githubusercontent.com` and `csrc.nist.gov`; the GitHub
API was blocked by the session scope. File sizes below were measured with `curl`. Nothing was
vendored.

| Source | Covers | Size (measured) | Licence | Plan |
| --- | --- | --- | --- | --- |
| **Wycheproof** (C2SP/wycheproof, `testvectors_v1/*.json`) | HMAC SHA-256/384, HKDF SHA-256/384, AES-GCM, ChaCha20-Poly1305, X25519, ECDH P-256/P-384, ECDSA P-256/SHA-256, P-384/SHA-384 and P-384/SHA-256, Ed25519, RSA PKCS#1 verify (2048, 3072, 4096, several hashes), RSA-PSS | e.g. `aes_gcm` 213 KB, `chacha20_poly1305` 241 KB, `x25519` 254 KB, `ecdh_secp256r1` 418 KB, `ecdh_secp384r1` 818 KB, `ecdsa_secp256r1_sha256` 327 KB, `ecdsa_secp384r1_sha384` 375 KB, `ed25519` 127 KB, `rsa_signature_2048_sha256` 211 KB, `rsa_pss_misc` 496 KB, `hmac_sha256` 69 KB, `hkdf_sha256` 93 KB. Roughly 5 to 8 MB for everything this plan needs. | Apache-2.0 (LICENSE fetched and read) | **Vendor** a pinned subset under each crate's `tests/vectors/` with the commit hash and per-file SHA-256 in a manifest, plus the licence. Wycheproof has no `ecdsa_secp256r1_sha384` file (404 when fetched), so P-256/SHA-384 needs other vectors. |
| **NIST CAVP** (SHAVS, HMAC, GCM, ECDSA SigVer, KAS) | SHA-2, HMAC, AES-GCM, ECDSA verify, ECDH | zip downloads; `shabytetestvectors.zip` reachable | US Government work, public domain | Vendor only what Wycheproof does not give (SHA-2 byte-oriented and long messages); convert to a compact text form with a script, keep the script. |
| **RFC vectors** | 4231 (HMAC), 5869 (HKDF), 7748 (X25519), 8032 (Ed25519), 8439 (ChaCha20-Poly1305), 6979 (RFC 6979 ECDSA), 8017 (RSA); McGrew-Viega GCM | tiny | IETF Trust, example data intended for reuse | Hand-copy into tests. |
| **RFC 8448** | TLS 1.3 traces | already in `rusty_tls` tests | | Gives an end-to-end KAT through the key schedule and record layer once a backend seam exists. |
| **`ring` as oracle** | everything it implements | dev-dependency, allowed by ADR-0002 Tier S | ISC-style | Differential tests (random inputs, both directions), and `ring`'s own signing output verified by our verify. |
| **x509-limbo / BetterTLS** | certificate path | not needed for primitives | | Out of scope here; part of the TLS track. |

Wycheproof is the vector set aimed at exactly the encoding and edge-case bugs that matter for
stage 2 (DER malleability, non-canonical scalars, small-order points). Fuzzing here is a stable,
seeded, deterministic harness; no coverage-guided fuzzer is installed (no nightly).

## 7. Stage plan

### 7.1 Crate layout (proposal; names are the owner's)

Tier S, `layer = "foundation"`, no dependencies, `forbid(unsafe_code)`, `ring` and vector
data as dev-dependencies only. One concern per crate, composed:

| Crate | Contents | Stage |
| --- | --- | --- |
| `rusty_ct` | masks, select/eq/lt/swap, `Secret<T>` newtypes, a conformance harness for the evidence tools (not a runtime dependency of anything else) | 0 |
| `rusty_sha2` | SHA-256/384/512, HMAC, HKDF | 1 |
| `rusty_rsa` (additive modules) or a new crate | fixed-width Montgomery, PKCS#1 v1.5 and PSS verify | 2 (decision: extend the existing shared crate additively, or a new one; I recommend extending, without changing existing public API) |
| `rusty_ec` | field arithmetic (constant time from the start so stages 2 and 6 share it), P-256, P-384, Ed25519, X25519, ECDSA | 2, 4, 6 |
| `rusty_aead` | ChaCha20-Poly1305, AES-GCM | 3, 5 |

Rationale for building the field arithmetic constant time even for verification: it costs
little, lets stage 6 reuse it, and the only variable-time pieces (scalar multiplication for
verify) get the `_vartime` suffix and are greppable. Heap secrets reuse `rusty_crypto_key`;
stack secrets need an additive `wipe` there (decision).

### 7.2 Stages, evidence bars, effort

Effort is focused engineering days, plus or minus 50 percent, excluding review wait.
"Abandon value" is what you keep if the stage is the last one done.

| Stage | Scope | Evidence bar (proposed) | Effort | Abandon value |
| --- | --- | --- | --- | --- |
| **0** Harness | `rusty_ct`; vector loader; differential driver vs `ring`; stable seeded fuzz harness; valgrind, disassembly and dudect scripts; a scheduled CI job | Tools catch the planted leaks from section 4.2 in CI | 5 to 7 | Reusable by any future crypto |
| **1** SHA-2, HMAC, HKDF | SHA-256/384/512, HMAC, HKDF, constant-time tag compare | CAVP + RFC 4231/5869 + Wycheproof; differential vs `ring` on 10^6 random inputs incl. block boundaries and lengths 0 to 300; fuzz; zeroize on drop | 3 to 4 | `rusty_oauth`/`rusty_rdp`/`rp-router` can drop private HMAC copies and `ring::hmac` |
| **2** Signature verify | RSA PKCS#1/PSS 2048 to 8192, ECDSA P-256/P-384, Ed25519 | Wycheproof (all verify files); differential vs `ring` incl. **every accept/reject divergence listed and classified** (stricter / looser / equal), ring's quirks from section 2.1 matched or documented; fuzz of DER and point parsing; RSA-4096 and 8192 timing within 5x of `ring` | 20 to 25 | `rusty_tls` verify path off `ring` (F7 scope kept); also removes the 200x slow RSA path for `rusty_oauth` |
| **3** ChaCha20-Poly1305 | AEAD, 12-byte nonce | RFC 8439 + Wycheproof + differential; section 4.4 methods 1 to 3 clean; reviewed once | 3 to 4 | Fast portable AEAD, ChaCha-first client policy possible |
| **4** X25519 | RFC 7748 | Wycheproof + RFC vectors + differential; all-zero output rejected as `ring` does; methods 1 to 4 clean; independent review. **Evaluate vendoring fiat-crypto generated field code** (formally verified, straight-line, MIT/Apache/BSD; COPYRIGHT file reachable, terms not re-checked) as the strongest correctness evidence on offer | 3 to 4 (+2 to evaluate fiat) | Ephemeral key exchange off `ring` for 25519 |
| **5** AES-GCM | Fixsliced/bitsliced AES-128/256, constant-time GHASH (no tables) | Wycheproof + CAVP GCM + differential; no table lookup indexed by secret (disassembly script) ; methods 1 to 4 clean; review. **Throughput is 60 to 150x below `ring` unless intrinsics are approved.** | 8 to 12 (+5 to 8 for a reviewed intrinsics module, only if approved) | Needed before any ring-free client; useless alone |
| **6a** ECDH P-256/P-384 | secret scalar multiplication, complete formulas, constant-time table selection, on-curve and infinity checks | Wycheproof ECDH (incl. invalid-curve and twist cases); differential; methods 1 to 4; review | 8 to 10 | **Client-only engine ring-free (with 1 to 5)** |
| **6b** Signing | ECDSA P-256/P-384 (RFC 6979 and hedged nonce), Ed25519, PKCS#8 parsing | Wycheproof; differential both directions (their signatures verify under ours and vice versa); nonce bias test; methods 1 to 4; review | 6 to 8 | Server role and client certificates off `ring` except RSA |
| **7** RSA sign | constant-time modexp, CRT, blinding, fault check, PKCS#8 RSA parse | As 6b, plus blinding tested, plus verify-after-sign | 15 to 25 | **Recommend not doing.** |

Totals: stages 0 to 3 about 31 to 40 days; through 5 about 40 to 56 (plus intrinsics);
through 6 about 54 to 74; with 7 about 70 to 100. Add 2 to 4 weeks of external review for
anything past stage 3. These are separate from, and additional to, the 45 to 70 days the TLS
assessment already estimated for making the engine the default.

Each stage is abandonable: a stage ships only when its bar is met and is not wired into
`rusty_tls` without the owner's approval.

## 8. Risks

1. **The invisible failure.** Vectors, differential tests and fuzzing cannot see timing or
   cache leaks; the bar in 4.4 reduces but cannot remove the risk, and `ring`'s assembly comes
   with years of public scrutiny that this code will not have.
2. **Compiler drift.** Rust gives no constant-time guarantee; LLVM may turn a mask into a
   branch. The disassembly gate pins one toolchain; every `rustc` bump needs a rerun.
3. **Variable-latency hardware.** Multiplication and division timing differs on some cores.
   Target x86-64 and aarch64 application cores only; document it. Never `/` or `%` on secrets.
4. **Behaviour drift from `ring`** at the edges (section 2.1) turning into either rejected
   valid certificates or accepted invalid ones. Every divergence is a finding until classified.
5. **Zeroization needs `unsafe`.** The only way to wipe reliably in Rust is volatile writes;
   it stays confined to `rusty_crypto_key`.
6. **Performance cliff at stage 5** (section 5), and the engine's default suite order puts
   AES-GCM first.
7. **False sovereignty.** `ring` stays in the lockfile (section 2.2); `rusty_tls` stays Tier
   A while `rustls` is its default. Do not market stage completion as "ring-free".
8. **The existing variable-time signer** in `rusty_oauth` (ES256 over `BigUint`) already
   handles a secret key. Not in scope, but the owner should know it exists.
9. **Effort is understated if review finds structural problems**; stages 4 to 6 are where I
   would expect rework.
10. **Maintenance.** First-party crypto is now the owner's to patch for every future
    advisory.

## 9. Decisions for the owner (I make none of these)

1. Proceed past stage 2? (My recommendation: yes to 3 and 4 only after 1 to 2 land; 5 and
   beyond only after 4 and 5 below.)
2. Is `unsafe` allowed for AES-NI/PCLMUL (and a valgrind harness)? Recommendation: yes for a
   single reviewed module behind a runtime feature check, never elsewhere; otherwise accept
   stage 5 as client-WAN-only.
3. Does any stage replace `ring` in `rusty_tls`, and behind what? This needs a backend seam in
   `rusty_tls` (same function signatures, selected by an additional cfg next to
   `rusty_tls_handrolled`). I have not touched `rusty_tls`; the TLS-engine session owns it.
4. The constant-time evidence bar (section 4.4 is a proposal).
5. Layout: `rusty_rsa` extended additively vs new crate; additive `wipe` in
   `rusty_crypto_key`; vendoring fiat-crypto generated code for field arithmetic.
6. Stop point. My suggestion: stages 0 to 4 as a firm plan, 5 to 6a if intrinsics are approved,
   6b if a server role needs it, 7 never.

## 10. Draft ADR text (NOT ACCEPTED)

> **ADR-0005 (draft): First-party cryptographic primitives behind the native TLS engine**
> Status: **Proposed. Not accepted.** Would supersede section 6 of ADR-0002 only for the
> primitives and stages the owner approves.
>
> **Context.** The native engine uses `ring` for every primitive. ADR-0002 section 6 kept
> `ring` and required a separate issue and ADR to revisit. `ring` is a dependency of ten
> packages in the lockfile, so replacing it in `rusty_tls` does not remove it from the build.
> The primitives differ in what can be verified: hashes, HMAC, HKDF and signature
> verification handle only public data and can be validated with vectors, differential tests
> and fuzzing. Everything that handles a secret key can leak through timing or cache effects
> that none of those techniques can observe.
>
> **Decision (proposed).** Build first-party, dependency-free, `forbid(unsafe_code)` Tier S
> crates in stages. Stages 1 and 2 (SHA-2, HMAC, HKDF, signature verification) proceed on the
> evidence bar of vectors, Wycheproof and differential testing against `ring`, with every
> divergence classified. Stages that handle secrets (ChaCha20-Poly1305, X25519, AES-GCM, ECDH,
> ECDSA/Ed25519 signing) proceed one at a time, each only after: valgrind secret-taint checks,
> a pinned-toolchain disassembly check, a scheduled statistical timing test, and an
> independent review. RSA signing is not built. `ring` remains a dev-dependency oracle. No
> stage is connected to `rusty_tls` without a separate owner approval of a backend seam.
>
> **Consequences.** Auditability and a smaller native build, at the cost of slower portable
> code (notably AES-GCM unless intrinsics are approved), a maintenance burden, and weaker
> evidence than `ring` for secret-handling code. Claims are limited to "no leak detected by
> the listed methods on the listed toolchain". `rusty_tls` stays Tier A while `rustls` is its
> default.
>
> **Alternatives.** Keep `ring` (status quo; the earlier D2 recommendation). Replace only the
> public-data stages and keep `ring` for the rest (the recommended floor). Vendor generated,
> formally verified field arithmetic (fiat-crypto) instead of writing it.

## 11. What I did not verify

- No primitive was written; every native performance figure in section 5 is an estimate.
  Only the `ring` numbers and the `rusty_rsa` numbers are measured, once, on a noisy VM.
- The constant-time probe tested the tools on toy code, not on any real primitive.
- The disassembly script was not finished; I only showed the idea needs symbol-exact extraction.
- The `rusty_oauth` P-256 code and `rusty_rdp` HMAC were skimmed, not audited.
- `ring` behaviours were checked in source for: Ed25519 canonical `S`, RSA exponent range,
  PSS salt length, ECDSA DER and key encoding. Modulus rules and SHA-1 legacy handling were not.
- Wycheproof file availability and sizes were measured; contents and the exact commit to pin
  were not examined. fiat-crypto's licence terms and output quality were not checked.
- Licence statements other than Wycheproof's (read) come from general knowledge.
- Whether aarch64 timing behaves as assumed was not tested (x86-64 only).
- `rusty_tls#25` remains unread (see the TLS assessment).
