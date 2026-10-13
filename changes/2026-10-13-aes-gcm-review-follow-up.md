---
category: Fixed
changelog: AES-GCM review follow-up: wipe the remaining key-material copies, enforce the 2^61-1-byte AAD limit, restore the exhaustive S-box test, matched timing controls, corrected evidence accounting and statements.
---
## 2026-10-13 - AES-GCM review follow-up (review of #651)

- **Fixed (rusty_aead):** the replicated round-key copies in `Aes::expand`, the input and output block groups in `Aes::encrypt_blocks`, all eight hash-key lanes in `Gcm::new`, and `Gcm`'s hash key on drop are now wiped with `rusty_crypto_key::wipe`. `seal_in_place` and `open_in_place` refuse an AAD longer than 2^61 - 1 bytes (the specification's limit; GHASH writes its bit length into a 64-bit field), tested arithmetically at the boundaries. Temporaries inside the straight-line round functions are still not wiped.
- **Fixed (tests):** the exhaustive S-box-against-its-definition test, dropped in the `ct64` rewrite, is restored, with a SubWord test. Jump-count pins and taint results are unchanged.
- **Fixed (evidence):** matched A/A controls for the constructor-inclusive key tests and the invalid-tag open test, run before the test each calibrates; `collect.sh` now prints executed and skipped counts for the AES-GCM vectors. `docs/research/crypto-evidence/ERRATUM-2026-10-13-aes-record-accounting-and-baselines.txt` corrects the two failed AES records, which imply 316 executed cases (133 ran) and used an unmatched baseline; the raw records are unchanged and were not regenerated.
- **Fixed (docs):** statements that said nothing was implemented, nothing past step 1 had started, or that CI runs the timing series (it runs the taint modes and pins only).
- **Known limitations:** the AES types are exported unconditionally (a feature gate is an open API question for the owner); the ChaCha20-Poly1305 timing baselines have the same mismatch, unchanged; no new full evidence record was taken after these changes, so there is no full record for this head.
