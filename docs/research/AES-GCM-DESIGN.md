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
64-byte message (5 blocks) pays for a whole batch: 7 to 9 MB/s, about 90 to 100 times slower than `ring` at that size.
`open_in_place` also computes the first batch twice, which the measurement above does not include.

### Profile (2026-10-10, idle machine, release build, best of five; ignored tests `profile_stages` and `profile_ghash`)

Per 64-block batch (1,024 bytes), AES-128: 6,006 ns in total, i.e. 170 MB/s on its own (AES-256: 7,768 ns, 132 MB/s).
GHASH on its own: 252 MB/s (one field multiplication 62 ns). Combining them as `1 / (1/AES + 1/GHASH)` gives 101 MB/s
(AES-128) and 87 MB/s (AES-256), which matches the measured 103 and 86 MB/s, so the two halves account for the time.

| AES-128 stage | per batch | calls | total | share of 6,006 ns |
| --- | --- | --- | --- | --- |
| S-box (`sub_bytes`) | 278 ns | 10 | 2,780 ns | 46% |
| transposes (`to_planes` 644 + `from_planes` 647) | 1,291 ns | 1 | 1,291 ns | 21% |
| `mix_columns` | 99 ns | 9 | 891 ns | 15% |
| `add_round_key` | 52 ns | 11 | 572 ns | 10% |
| `shift_rows` | 37 ns | 10 | 370 ns | 6% |

(The stages timed in isolation add up to 5,902 ns against 6,006 ns for the whole, so nothing large is missing.)
Key expansion costs 661 ns per new key, and `Gcm::new` spends one more batch (about 6 µs) on the hash key.
At bulk sizes AES is about 60% of the time and GHASH about 40%.

**What this says about the design, including a correction to an earlier statement.**
- The S-box circuit is already near what scalar code can do (about 1,800 boolean operations in 278 ns, roughly three
  per cycle). More speed from this layout needs SIMD, which means intrinsics or an unstable portable-SIMD feature.
- **A narrower batch does not help small messages.** An earlier version of this note, and the report that went with
  it, suggested a batch of 8 or 16 blocks. That was wrong: with one block per bit of a word, the number of boolean
  operations is the same whatever the word width, so a narrower word costs the same per batch and only lowers
  throughput. What would help is a different bit layout, the one BearSSL calls `ct64`, which packs the 16 byte
  positions of a block into each plane word so that one batch is 4 blocks, not 64, at the same cost per block. That is
  a re-layout of the state, `shift_rows` and `mix_columns` (they become rotations inside words), not a parameter. It
  would also shrink the transposes (21% of the time here). **This is a hypothesis, not a measurement:** I estimate a
  64-byte message could get several times faster, and bulk speed would be unchanged or a little better.
- GHASH has room: the hash-side reversed operands could be precomputed once per key, and several blocks could share
  one reduction. I would not expect more than about 1.5 to 2 times from GHASH, and nothing here changes the
  conclusion that portable AES-GCM stays tens of times slower than `ring`.
- Realistic ceiling for the portable path, as an estimate: roughly 150 to 200 MB/s bulk after both optimisations.

### The `ct64` re-layout experiment (2026-10-10; run after the profile)

The profile suggested repacking the bit planes so that a batch is a few blocks, not 64. I tried it: eight planes of
`u64`, bit `16b + p` of plane `k` holding bit `k` of byte `p` of block `b`, so one S-box circuit handles four blocks
(`aes.rs`). The previous 64-block layout was kept as a test-only oracle while the new core was checked against it, then removed (see "Decision" below).
Two plane sets run side by side per call (8 blocks), which measured best of one, two and four (AES-128 alone: 112,
132 and 120 MB/s).

**Correctness:** the new core matches the old one on 300 random keys and 64 random blocks each (both key sizes), the
FIPS 197 vectors pass, the transposes and the plane layout match their definitions, and the whole GCM suite (spec
vector, `ring` differential, tamper tests) passes unchanged.

**End-to-end seal throughput, MB/s (single run each, same idle machine; `ring` varied by up to 1.6 times between runs,
so compare the absolute columns, not the ratios):**

| Size | AES-128 wide | AES-128 ct64 | AES-256 wide | AES-256 ct64 |
| --- | --- | --- | --- | --- |
| 64 B | 8.8 | 41.4 | 7.4 | 33.0 |
| 1 KiB | 61.6 | 73.7 | 51.7 | 61.6 |
| 16 KiB | 90.7 | 81.0 | 83.9 | 65.2 |
| 16 MiB | 102.7 | 79.2 | 86.1 | 69.3 |

**Reading:** small messages are about 4.7 times faster (64 B), 1 KiB about 20% faster, and bulk is 11% to 22% slower
(AES-128 at 16 KiB 11% slower, AES-256 at 16 KiB 22% slower, 16 MiB 23% and 20% slower). The cause is in the linear
layers: `shift_rows` and `mix_columns` now move bits within words and cost about five times more per block than the
wide layout's word permutation, while the S-box got faster. GHASH (252 MB/s) is now a third of the time at bulk sizes.
Not repeated, so run-to-run variance of these figures is unknown.

**Options, none built:** (1) keep ct64 only, with one AES core to review: better for handshake messages, tickets and
typical small records, 10 to 20% worse on 16 KiB records and bulk. (2) Keep both layouts and choose by length: best at
every size, but two AES cores to review and to put through the evidence bar. (3) Keep the wide layout only: best
for bulk, 4.7 times worse on 64-byte messages. **My recommendation is (1)**: TLS records are at most 16 KiB, the
difference at that size is 11 to 22%, and a second core doubles the review and evidence work for a gain the
client role is unlikely to notice.

**Decision (owner, 2026-10-10: "keep ct64"):** option (1), `ct64` only. The 64-block layout and its equivalence test were
removed once the new core had independent oracles: FIPS 197, the GCM specification vector, `ring`, and Wycheproof (below).
The throughput figures above are unchanged by the removal; the old layout's numbers are history.

**Wycheproof (`tests/aes_gcm_vectors.rs`, file pinned at the same commit as the ChaCha vectors, sha-256 in `MANIFEST.txt`):**
316 cases, 133 executed (AES-128 and AES-256, 96-bit IV, 128-bit tag: 79 valid, 54 invalid), all pass. The other 183 cannot be
run through the typed API and are counted, not dropped: 103 use AES-192 and 80 use an IV that is not 96 bits (of those,
some are valid encryptions under the standard, so they are skipped rather than asserted invalid). The totals are pinned in
the test. A mutation check, dropping the tag mask and then breaking one GHASH reduction shift, makes the test fail both times.
Not done: other specification vectors (the GCM specification's own cases, NIST CAVP, whose terms for vendoring are unchecked).

**Constant-time evidence for the new core:** the valgrind taint run was repeated on the new code (seal 0 reports,
open exactly 1 (the public verdict), planted control caught), and the existing `rusty_aead` checks still pass. The
earlier taint result applied to the old layout only. Still one build and one run per mode.

**Early constant-time signal (valgrind taint, `examples/taint_aes.rs`; one build, one run each, x86-64):** key and
plaintext marked secret, 2000-byte plaintext (covers the first and later batches). `gcm128-seal` and `gcm256-seal`: 0
reports. `gcm128-open`: exactly 1 report, a conditional jump in `Gcm::open` (the public tag verdict). The planted
control, a table lookup indexed by a secret byte, is reported ("use of uninitialised value of size 8"), so the tool
sees this leak class here. **Not yet done:** jump-count pins, timing tests, CI wiring, and everything that makes this
more than one run. This is not a constant-time claim.

**Gate result:** the step 1 gate asked for a measured MB/s to be reported to the owner before more is built. It is
reported above. Nothing past step 1 has been started.

## 9. Step 4 results: constant-time evidence (2026-10-10; preliminary; evidence, not proof)

**What was added** (`rusty_aead`; CI runs all of it through the existing `crypto-constant-time` job):
- Taint modes in `examples/taint_aes.rs`: `gcm128-seal` and `gcm256-seal` must report nothing, `gcm128-open` exactly
  one report in `Gcm::open` (the public tag verdict), and `control-table` (a secret-indexed table lookup, the leak
  class table-based AES has) must be reported. Result: 0, 0, 1 in `Gcm::open`, 1.
- Exact jump-count pins (`scripts/disasm_limits_aes.txt`): `sbox`, `shift_rows`, `mix_columns`, `to_planes`,
  `from_planes`, `sub_word`, `bmul64`, `clmul64`, `gf_mul` are straight-line (0); `Aes::encrypt_blocks` has 4 and
  `Aes::expand` 17, each read in the disassembly and found to compare a public counter or index (loop entry and
  back-edge, round-key bounds checks, the key-schedule word index). The functions are `#[inline(never)]`, and the
  key schedule now counts its word index instead of dividing, so there is no division anywhere in the audited set.
  `ghash::ghash` and `Gcm::{ctr, seal, open}` branch on the public message length and are not pinned.
- Five timing tests (AES-128 and AES-256 key classes, plaintext classes, tag-mismatch position, A/A baseline), 50,000
  samples each, in `collect.sh`.

**The checks were tested.** Planting a real branch on secret data in `mix_columns`: the taint run reports it (4
reports in each seal mode) and the pin fails (1 against 0). Planting a secret-indexed table lookup in the key
schedule: the taint run reports it (1 and 2 reports), and the pin does not, as expected, since a lookup adds no jump.
**My first two attempts at these mutations changed nothing observable** (`^= 0`, `& 0`), the compiler removed them, and
the checks passed; that was a bad mutation, not a good check, and they were redone with effects the compiler cannot
remove. The same applies to any future mutation check here.

**Two full `collect.sh` records were produced, and both FAILED, for reasons unrelated to AES timing.**
- `EVIDENCE-2026-10-10-aes-gcm-run1-FAILED.txt` (clean tree at `122a00b7`): `clippy rusty_aead` failed on 1.98.1 and
  on stable, three `needless_range_loop` lints in test code. All timing rows were within their allowance. The lints
  had been missed locally because the check piped coloured clippy output through `grep '^error'`, which never matched;
  it had also let them reach the pushed branch. Fixed in `82dd3177`, and clippy was re-run with `--color never`,
  both toolchains and the exit code checked, for every crate touched in this work.
- `EVIDENCE-2026-10-10-aes-gcm-run2-FAILED.txt` (clean tree at `82dd3177`): clippy clean, taint, pins, vectors and
  self-tests passed, and **one timing row failed: ECDH P-256 sparse-vs-dense, 2 alarms of 40 (|t| 4.98 and 5.19)
  against an allowance of 1**. That is the watch item from #576, in code this work does not change. A diagnostic that
  reran it at 4,000 to 256,000 samples found no growth in |t| (0.77, 0.38, 0.41, 0.63);
  `EVIDENCE-2026-10-10-ecdh-p256-sample-scaling.txt` has the numbers, the full tally (3 alarms in 240 leak-test runs
  against 0 in 240 A/A runs) and its cautions. **No third full record was run and no threshold or allowance was
  changed**: running until a pass would be the sampling this note's rules forbid, so there is no passing full record
  for this commit, and whether to take one is the owner's call.

**AES timing rows in run 2** (all within allowance, 0 alarms each): A/A AES-128 max 4.07; AES-128 key classes max
4.37; AES-256 key classes max 2.90; plaintext classes max 2.44; tag-mismatch position max 3.05. Run 1: AES-128
plaintext classes 1 alarm of 40 (|t| 4.54, within the allowance of 1; A/A 0 alarms at that time). Alarms elsewhere in
the two records: HMAC-SHA512 key classes 1 (4.65) and 0 of 40 in one record each, HMAC-SHA256 key classes 1, ChaCha20-
Poly1305 open 1, all within allowance. Across the four full records so far HMAC-SHA512 key classes has alarmed 2, 0,
1 and 1 times (see the plan, section 13.4).

**A defect in the evidence tooling was found on the way** and is recorded in
`ERRATUM-2026-10-10-timing-test-filter.txt`: the collector selected tests by libtest's substring filter, so the
"A/A X25519" row in the two 2026-10-09 ECDH records cannot be attributed to X25519 (verdicts unchanged). Fixed with
`--exact` and a regression test; the new AES test names would have collided with the ChaCha ones.

**Variance of the speed figures.** In the first full record the same throughput test gave 61.8 MB/s for AES-128 at 16
MiB and 35.8, 62.1 and 69.0 at 64 B, 1 KiB and 16 KiB (AES-256: 58.5, 30.3, 50.3, 56.7), against 79.2, 41.4, 73.7 and
81.0 in section 8. So the figures in this note are good to roughly plus or minus 20%, not better.

**Still not done:** other specification vectors, a randomised differential suite, repeat taint runs on other
compilers (the `ct_check.sh` loop for `rusty_pk` runs two toolchains; this crate's script runs the default one),
timing on any machine but this one, and any independent review.
