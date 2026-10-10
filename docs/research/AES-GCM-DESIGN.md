# AES-GCM for `rusty_aead`: design note (draft, no code)

Date: 2026-10-10. Status: **draft for the owner; nothing is implemented.** Plan context:
`CRYPTO-REPLACEMENT-PLAN.md` section 13.1 gap 3, decisions D1 and D2. The freeze was reopened for this gap only
(plan section 0, "Owner decisions (2026-10-10)"). The `rusty_tls` seam and signing stay closed. Numbers marked
*estimate* are not measurements; the first step below produces the measurement.

## 1. What and why

AES-128-GCM and AES-256-GCM with a 12-byte nonce and a 16-byte tag: the shape `rusty_tls` uses at its `ring` call
sites (record protection, `record.rs`; session tickets, `ticket.rs`). RFC 8446 requires `TLS_AES_128_GCM_SHA256`,
so the native engine cannot be `ring`-free without it. Out of scope: other nonce or tag lengths, AES-192, AES-CCM,
any `rusty_tls` change, suite-order policy.

## 2. The decision (D1)

| | A. Portable, table-free | B. AES-NI + PCLMULQDQ intrinsics |
| --- | --- | --- |
| `unsafe` | none; `#![forbid(unsafe_code)]` stays | one reviewed module, runtime feature check; needs the crate attribute relaxed for that module |
| Throughput | *estimate* 60 to 150 MB/s (`ring`: 9.1 GB/s AES-128, 7.6 GB/s AES-256 on this machine) | near `ring` |
| Side channels | no secret-indexed memory, no secret branches by construction | hardware AES is constant time; the wrapper code still needs the same checks |
| Targets | every target | x86-64 and aarch64 only; a portable fallback is still needed |
| Effort (plan estimate) | 8 to 12 days | A plus 5 to 8 days |
| Status | can start now | needs the 2026-10-08 approval reconfirmed (D1 is open) |

**Recommendation: build A first, and decide B later with a measurement in hand.** A is required anyway as the
fallback for B, it adds no `unsafe`, and it settles whether 60 to 150 MB/s is acceptable for the client role. B is a
separate, separately approved module.

## 3. Design of A

- **Placement:** `crates/foundation/rusty_aead`, new modules `aes`, `ghash`, `gcm`; public types `Aes128Gcm` and
  `Aes256Gcm` with the same `new(&key)`, `seal_in_place(nonce, aad, buf) -> tag` and `open_in_place(nonce, aad, buf,
  tag)` shape as `ChaCha20Poly1305`. No new crate, no new dependency.
- **AES core:** bitsliced/fixsliced AES (Adomnicai and Peyrin, 2020) on 64-bit words with the Boyar-Peralta S-box
  circuit. The key schedule uses the same constant-time S-box. No lookup table, no branch on key or data. Round
  keys are held in the cipher struct and wiped on drop, as `ChaCha20Poly1305` wipes its key.
- **GHASH:** carry-less multiplication built from integer multiplies with masked "holes" (the BearSSL
  `ghash_ctmul64` technique), so no table is indexed by the hash key or the data. **Assumption, stated rather than
  hidden:** 64-bit integer multiply takes data-independent time on the targets we support (x86-64 and aarch64
  application cores). That is not true of some small cores, so it is recorded as an assumption of this design.
- **GCM:** 96-bit IV only, so `J0 = IV || 0^31 || 1`; 32-bit counter increment; inputs longer than the GCM limit
  (2^36 - 32 bytes) are refused; the tag is checked with `rusty_crypto_key::constant_time_eq` and the buffer is left
  unchanged on failure, exactly as `open_in_place` does for ChaCha20-Poly1305.
- **Nonce reuse:** documented on the type as for ChaCha20-Poly1305, and worse here (it leaks the GHASH key). No
  nonce generation in the crate.

## 4. Evidence bar (same standing rules as plan 13.3)

1. **Vectors:** Wycheproof `aes_gcm_test.json` vendored and pinned in `MANIFEST.txt` (license file already present
   for the ChaCha vectors); the GCM specification test cases (McGrew and Viega); NIST GCM vectors if their terms
   allow vendoring (to be checked, not assumed). Wycheproof cases outside the typed API (IV length other than 96
   bits, tags shorter than 16 bytes) are counted and asserted invalid or unsupported, as the ChaCha test does for
   non-96-bit nonces.
2. **Differential against `ring`,** both directions, over lengths 0 to 300, AAD lengths 0 to 64, in-place and block
   boundaries, plus random keys; identical accept/reject verdicts for tampered tags, ciphertexts and AAD.
3. **Constant-time evidence** (evidence, not proof; x86-64, one VM, as for the other crates):
   - valgrind taint with the key and plaintext marked secret before use, covering key schedule, seal and open. The
     method reports a secret-dependent branch *and* a secret-dependent memory address. I confirmed on 2026-10-10
     that the repo's `valgrind_selftest.sh` passes here, including its `leaky-index` control (a secret-indexed
     table lookup), which is the leak class table-based AES has. The only allowed report is the public tag verdict
     in `open_in_place`, as for ChaCha.
   - **A planted control for this crate:** a table-based S-box variant in the taint example that the check must
     flag, so a clean result means the tool could see this class of leak here.
   - pinned conditional-jump and division counts for the AES round and GHASH functions (expected zero in the
     round function, exact numbers pinned after the first build, `#[inline(never)]` where inlining would move them,
     as was needed for `Modulus::add`). The disassembly script cannot show that no memory access depends on a
     secret; the taint run is what covers that.
   - dudect-style timing: key classes and plaintext classes, with an A/A baseline per test and the existing
     threshold and allowance rule, unchanged.
4. **Compiler risk:** LLVM can turn straight-line bit operations into branches. The jump-count pins are there to
   catch exactly that, per toolchain; a toolchain bump re-reads them.
5. **Review:** the owner alone (D4, decided 2026-10-10). The review checklist I would hand over: the S-box
   circuit transcription, the fixslice round schedule, the GHASH hole masks, counter and length handling, wiping.

## 5. Order of work and gates

| Step | Deliverable | Gate before the next step |
| --- | --- | --- |
| 1 Spike (about 1 day, *estimate*) | AES-128 block core and GHASH, tested against FIPS 197 and one GCM vector, plus a throughput measurement | **Measured MB/s reported.** If it is far below the 60 to 150 estimate, stop and tell the owner before building more. |
| 2 | AES-256, full GCM, API, wiping, docs | vectors and differential pass |
| 3 | Wycheproof and spec vectors pinned; differential suite | all pass, case counts reported |
| 4 | Taint modes, planted control, jump-count pins, timing tests, CI wiring in the existing `crypto-constant-time` job | control flagged, checks clean, evidence record kept (failures kept too) |
| 5 | Plan section 13.4-style results; PR | owner review |

No step touches `rusty_tls`. Throughput below `ring` by 60 to 150x means the engine should order ChaCha20-Poly1305
before AES-GCM when only the portable path exists. That is a `rusty_tls` policy and a ClientHello fingerprint
change, so it is for the seam work (gap 4, closed) and is flagged here only so it is not forgotten.

## 6. Open questions for the owner

1. **D1:** A first, as recommended? Or B directly (reconfirm the intrinsics approval)?
2. **Performance bar for A:** what MB/s makes A worth finishing? Proposal: report the step-1 number and let the
   owner decide, rather than fix a threshold now.
3. **Scope:** is 96-bit IV and 16-byte tag only acceptable? (It matches every `rusty_tls` call site.)
4. **Ticket cipher:** `rusty_tls`'s ticket key uses AES-256-GCM with a random nonce; with random 96-bit nonces the
   usual collision bound applies. Not a crate concern, but the seam work should look at it.

## 7. Risks

- Transcription errors in the S-box circuit or fixslice schedule: vectors and differential tests cover most; the
  review checklist names the rest.
- The multiply-time assumption in section 3 may not hold on a target we later add.
- A toolchain change can reintroduce branches: jump-count pins fail loudly, but only on the checked target.
- Throughput may make the native engine unsuitable for bulk transfers; step 1 measures this before most of the
  work is spent.

## 8. Step 1 results (2026-10-10; spike, preliminary)

Built per section 3: `aes.rs` (bitsliced AES-128/256, 64 blocks per batch), `ghash.rs` (table-free GHASH),
`gcm.rs` (`Aes128Gcm`, `Aes256Gcm`), all in `rusty_aead`, no `unsafe`, no new dependency.

**Correctness (what was run):** the S-box circuit matches its mathematical definition for all 256 inputs; the FIPS 197
appendix C vectors pass for AES-128 and AES-256 (this also covers the key schedule); GHASH matches a bit-by-bit
implementation of SP 800-38D algorithm 1 on edge values and 2000 pseudorandom pairs; GCM specification test case 4
passes; seal and open agree with `ring` over lengths 0 to 130 plus 255, 256, 1007 to 1009, 1023 to 1025, 2047, 2048,
4096 and 16384, AAD lengths 0, 1, 13, 16, 17 and 64, both key sizes, tag and ciphertext byte-for-byte; tampered tag,
ciphertext, AAD, nonce and a truncated tag are rejected and leave the buffer unchanged. **Not yet run:** Wycheproof and
the other specification vectors, a randomised differential suite, the GCM length limit, and any review.

**Throughput (this machine, idle, `--release`, best of five; `ring` measured in the same run):**

| Size | AES-128-GCM here | `ring` | AES-256-GCM here | `ring` |
| --- | --- | --- | --- | --- |
| 64 B | 8.8 MB/s | 806 MB/s | 7.4 MB/s | 707 MB/s |
| 1 KiB | 61.6 | 5,661 | 51.7 | 4,957 |
| 16 KiB | 90.7 | 8,970 | 83.9 | 6,442 |
| 16 MiB | 102.7 | 8,971 | 86.1 | 7,726 |

Bulk throughput (86 to 103 MB/s, 77 to 99 times slower than `ring`) is inside the plan's estimate of 60 to 150 MB/s.
**New finding: small messages are much worse than that estimate implies.** A batch always processes 64 blocks, so a
64-byte message (5 blocks) pays for a whole batch: 7 to 9 MB/s, about 90 to 100 times slower than `ring` at that size and
more in absolute terms than the bulk ratio suggests. Not profiled: how the time splits between AES and GHASH, and how
much a narrower batch (for example 8 or 16 blocks) would help small records. `open_in_place` also computes the first
batch twice (once for the tag mask, once for the data), which the measurement above does not include.

**Early constant-time signal (valgrind taint, `examples/taint_aes.rs`; one build, one run each, x86-64):** key and
plaintext marked secret, 2000-byte plaintext (covers the first and later batches). `gcm128-seal` and `gcm256-seal`: 0
reports. `gcm128-open`: exactly 1 report, a conditional jump in `Gcm::open` (the public tag verdict). The planted
control, a table lookup indexed by a secret byte, is reported ("use of uninitialised value of size 8"), so the tool
sees this leak class here. **Not yet done:** jump-count pins, timing tests, CI wiring, and everything that makes this
more than one run. This is not a constant-time claim.

**Gate result:** the step 1 gate asked for a measured MB/s to be reported to the owner before more is built. It is
reported above. Nothing past step 1 has been started.
