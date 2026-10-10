---
category: Added
changelog: AES-GCM spike in `rusty_aead` (bitsliced AES, table-free GHASH): correct against ring, about 100 MB/s bulk, slow for small messages; not wired into anything.
---
## 2026-10-10 - AES-GCM spike in `rusty_aead` (gap 3, step 1; preliminary)

- **Added (rusty_aead):** `Aes128Gcm` and `Aes256Gcm` (96-bit nonce, 16-byte tag), built on a bitsliced AES core (Boyar-Peralta S-box, 64 blocks per batch) and a table-free GHASH made from integer multiplies. No `unsafe`, no new dependency. Exported but used by nothing; `rusty_tls` is untouched.
- **Verified (run):** S-box against its definition for all 256 bytes, FIPS 197 vectors, GHASH against the specification algorithm, GCM spec test case 4, differential tests against `ring` over many lengths, AAD sizes and both key sizes, tamper rejection leaving the buffer unchanged, and one valgrind taint run per mode with a planted secret-indexed-lookup control that is caught (`examples/taint_aes.rs`). The existing `rusty_aead` constant-time check and jump-count pins still pass.
- **Measured:** 86 to 103 MB/s on 16 KiB to 16 MiB messages, 77 to 99 times slower than `ring`; 7 to 9 MB/s on 64-byte messages (a batch always processes 64 blocks). Details in `docs/research/AES-GCM-DESIGN.md` section 8.
- **Known limitations:** spike. No Wycheproof or other specification vectors yet, no randomised differential suite, no jump-count pins or timing tests for the AES code, no CI wiring for `taint_aes`, `open_in_place` computes the first batch twice, the time split between AES and GHASH is not profiled, the GHASH design assumes data-independent 64-bit multiply, nothing is independently reviewed, and none of this is a constant-time claim.
