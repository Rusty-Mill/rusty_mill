---
category: Fixed
changelog: rusty_pk verdict_sites.py closes the valgrind report files it reads.
---
## 2026-11-03 - verdict_sites.py closes the files it reads

- **Fixed:** `anchors()` and `reports()` in `crates/foundation/rusty_pk/scripts/verdict_sites.py`
  read through `with open(...)`, so they no longer raise `ResourceWarning` under `-W error` or
  dev mode, or leak handles on interpreters without refcounting. A test checks no
  `ResourceWarning` is emitted. No change to what the checker accepts or rejects.
