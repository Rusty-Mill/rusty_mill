---
category: Fixed
changelog: run_batch.ps1 no longer scores the *.pads.jsonl sidecars as captures, which aborted the batch under Windows PowerShell.
---
## 2026-10-21 - rb_tape_bot: batch scorer skips the pad sidecars

- **Fixed:** `run_batch.ps1` found captures with the wildcard `<name>_run*.jsonl`, which also matches the
  `<name>_run<k>.pads.jsonl` sidecar `rb_run_tapes` writes beside each capture (ADR-0071). The sidecar was passed to
  `rb-verify` as a capture, which rejected it (`missing field timestamp_secs`). Under Windows PowerShell 5.1, the
  documented `powershell -File` entry point, that stderr output is a terminating error, so the batch exited 1 after
  capturing everything and wrote no `results.md`; seen on the 2.78 smoke test. PowerShell 7 carried on and
  reported a spurious failed run. The scorer now takes exactly `<name>_run<digits>.jsonl`.
- **Added:** the `rb-tape-verifier` CI job puts a sidecar next to the fixture capture and also scores the batch with
  `powershell -File` (Windows PowerShell 5.1). Before the fix that run exits 1; the existing PowerShell 7 run passes
  either way, which is why the bug was not caught.
- **Known limitations:** under Windows PowerShell 5.1 any other stderr output from `rb-verify` still aborts the
  batch; this fixes the one known cause, not the class.
