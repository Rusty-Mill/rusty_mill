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
- **Added:** `test-run-batch.ps1` case `sidecar`: a batch with captures and sidecars is scored with a compiled
  stand-in for `rb-verify` that logs each `--against` path, and only the `_run<k>.jsonl` captures may appear. The old
  scorer logs the sidecars too. Under pwsh the report is identical either way, so the test counts calls instead.
  It needs `rustc`, which the `rb-tape-verifier` job already installs.
