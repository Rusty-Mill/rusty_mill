---
category: Fixed
changelog: run_batch.ps1 no longer passes the *.pads.jsonl sidecars to rb-verify as captures.
---
## 2026-10-21 - rb_tape_bot: batch scorer skips the pad sidecars

- **Fixed:** `run_batch.ps1` found captures with the wildcard `<name>_run*.jsonl`, which also matches the
  `<name>_run<k>.pads.jsonl` sidecar `rb_run_tapes` writes beside each capture (ADR-0071). The sidecar was passed to
  `rb-verify` as a capture, which rejected it (`missing field timestamp_secs`). Under pwsh that extra call was
  recorded as a failed run and left out of the report, so it cost a wasted `rb-verify` call and a misleading failed
  run; it is not known to change a score. The scorer now takes exactly `<name>_run<digits>.jsonl`.
- **Known limitations:** no regression test. Under pwsh the old and new scorer produce the same report, so a test
  through `results.md` cannot tell them apart; a test would have to count the `rb-verify` calls.
