---
category: Fixed
changelog: rusty_orch lens subcommands no longer panic when stdout is closed; the report goes to stderr instead.
---
## 2026-11-17 - No panic when stdout is closed

- **Fixed:** `rusty_orch lens ...` wrote its report with `print!`, which panics with "Broken pipe" when stdout is gone. In the first #1166 smoke run the terminal closed while `lens run` was working; the run finished its passes, panicked at the final print, and the whole run record was lost. The report (and `--help` text) is now written through one helper that falls back to stderr, and the exit code stays that of the work.
- **Known limitations:** other subcommands (`inspect`, `maintenance`, `watch`, the goal runner) still use `println!` and can panic the same way; only the `lens` family is fixed here.
