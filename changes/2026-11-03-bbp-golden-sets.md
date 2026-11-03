---
category: Added
changelog: rusty_bbp_host golden sets (#597 slice 5) - signed MANIFEST/SIGNOFF check (bad_golden), and bbp golden-run as the golden: profile's case runner.
---
## 2026-11-03 - Golden sets (Let Em Loose #597, slice 5)

- **Added:** `rusty_bbp_host::golden`. A golden set (`cases/<case>/case.toml`, fixtures, `MANIFEST`, `SIGNOFF`) opens a task only when every file matches `MANIFEST` and `SIGNOFF` names its set digest; otherwise the open is refused `bad_golden` and nothing is written. Private fixtures are listed by absolute host path and hash, and are never committed.
- **Added:** `bbp open --golden DIR` and `bbp holdout --golden DIR` add a signed set to the holdout suite as a `golden:<intent>` profile. Its command is `bbp golden-run DIR`, the generic case runner: it re-checks the sign-off, runs each case in the candidate's tree, compares stdout exactly or as JSON (keys in any order), checks the exit code and `stderr_contains`, and prints verdicts only.
- **Added:** `Code::BadGolden` in `rusty_bbp`. `rusty_bbp_host` now depends on first-party `rusty_codec` for its TOML parser.
- **Added:** unit tests for sign-off, tampering, host-only fixtures and case judging, and a sandboxed host test (trace CL-90).
- **Known limitations:** CODEOWNERS on Factory's `golden/**` belongs to the Factory repository. `rusty_codec`'s TOML has no multi-line strings, so long expected outputs go in `stdout_file` or `json_file`.
