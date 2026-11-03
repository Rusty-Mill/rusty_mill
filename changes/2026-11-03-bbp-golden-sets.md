---
category: Added
changelog: rusty_bbp_host golden sets (#597 slice 5) - signed MANIFEST/SIGNOFF check (bad_golden), and bbp golden-run as the golden: profile's case runner.
---
## 2026-11-03 - Golden sets (Let Em Loose #597, slice 5)

- **Added:** `rusty_bbp_host::golden`. A golden set (`cases/<case>/case.toml`, fixtures, `MANIFEST`, `SIGNOFF`) opens a task only when every file matches `MANIFEST` and `SIGNOFF` names its set digest; otherwise the open is refused `bad_golden` and nothing is written. Private fixtures are listed by absolute host path and hash, and are never committed.
- **Added:** `bbp open --golden DIR` and `bbp holdout --golden DIR` add a signed set to the holdout suite as a `golden:<intent>` profile. Its command is `bbp golden-run DIR`, the generic case runner: it re-checks the sign-off, runs each case in the candidate's tree, compares stdout exactly or as JSON (keys in any order), checks the exit code and `stderr_contains`, and prints verdicts only.
- **Added:** `Code::BadGolden` in `rusty_bbp`. `rusty_bbp_host` now depends on first-party `rusty_codec` for its TOML parser.
- **Changed (review):** a case's `stdout_file`/`json_file` must be a relative path inside its case directory that `MANIFEST` lists, and is checked against its digest when read, so the oracle is signed like the cases. A holdout run is granted each private fixture file, never its directory. `golden-run` feeds a case's stdin on its own thread while draining its output, so a command that writes before reading all its input cannot deadlock.
- **Added:** `confined_golden_run_passes_and_reads_only_signed_fixtures` is a case (`golden-confined`) in the Linux confined acceptance lane. It needs a working sandbox and requires the golden run to pass, the signed fixture to be readable, and an unsigned file beside it to be unreadable. The default host test no longer reads an `Error` status as a pass.
- **Added:** unit tests for sign-off, tampering, host-only fixtures and case judging, and a sandboxed host test (trace CL-90).
- **Known limitations:** CODEOWNERS on Factory's `golden/**` belongs to the Factory repository. `rusty_codec`'s TOML has no multi-line strings, so long expected outputs go in `stdout_file` or `json_file`.
- **Fixed (review):** the isolation check covers each golden set's absolute fixture files as well as the suite root, at open, at `bbp holdout` and before every run, so a fixture inside the repository, the work root or a build read root is refused.
- **Fixed (review):** a case command ended by a signal fails its case instead of reporting exit 128. `case.toml` fields of the wrong type (`stdout = ["ok"]`, `exit = "1"`) and unknown keys fail the case instead of defaulting to an expectation that accepts anything.
