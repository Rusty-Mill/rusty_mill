---
category: Added
changelog: The `erosion` profile command, and claims tasks now need an erosion profile or a recorded `"erosion": "off"` opt-out to open.
---
## 2026-11-04 - Erosion profile: `erosion check` and the claims open rule

- **Added:** the `erosion` binary in `rusty_erosion`, slice 2 of the erosion gate (card #704, D41, D42, D82). `erosion check --base COMMIT` measures the whole Rust tree at the base commit and in the working tree, and prints one JSON line with both levels, the exact deltas, the three worst-affected functions and changed code it cannot measure. With `--report-only` it always exits 0, even when it cannot measure. With `--max-concentration-bp N --max-duplication-bp N` it exits 1 over a limit, which fails the profile through the existing fail-block. Its git calls ignore system and user configuration.
- **Changed:** `bbp` refuses a claims task `bad_claims_config(erosion)` unless the profile set has exactly one profile named `erosion` with a full-commit `--base` that matches any pinned `baseline`, or the new `"erosion": "off"` opt-out. The field is left out when absent, so existing profile digests are unchanged.
- **Known limitations:** nothing goes live yet. The report-only time box and its pause at expiry (D42) come in a later slice, as do the inbox detail (slice 3) and the cleanup trigger (slice 4). A whole-repo run takes about 45 s in a debug build, so a profile's limits need to allow it.
