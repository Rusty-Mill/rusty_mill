---
category: Added
changelog: "`rusty_orch lens run` drives one BBP claims task with the configured lenses, one `bbp mcp` session per lens, until a pass starts no job."
---
## 2026-11-04 - rusty_orch: `lens run`

- **Added:** `rusty_orch lens run <config> --bbp --dir --task --state --experiment [--rules] [--assertions] [--repo] [--max-passes]` (Let Em Loose #594).
  - It starts one `bbp mcp` session per lens and reads the task brief as each lens's intent.
  - Each lens's prefix is built from the brief, the optional rules file and the lens prompt (relative to the config).
  - It restores the experiment's meter from the orch state directory and saves the meter before and after every call.
  - It then runs passes. Each pass refreshes every lens's view and runs one round for that lens alone.
- **Stops when:** a pass starts no job, a round halts (task ceiling, unsaved meter, refused post) or `--max-passes` is reached (default 20). The pass cap stops a provider that keeps failing transiently from being retried forever. Each job's outcome is printed.
- **Calls real models.** Provider admission follows the operator's assertions, as `lens check` reports. Don't run it until a live run is authorised.
- **Known limit:** the scheduler's memory lives only for the process, so a second run asks blind passes again. Saving it is the next slice.
- `Registry` now implements `orch_lens::call::Models`. `Session::brief` reads the brief once per run.
