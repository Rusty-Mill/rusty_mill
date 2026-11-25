---
category: Added
changelog: rusty_orch: `lens untested` checks a frozen intent's Windows-only lines against the task's mapping file at open (#1405).
---
## 2026-11-25 - rusty_orch: open-time check for Windows-only intent lines (#1405)

- **Added:** `orch_lens::untested` and `rusty_orch lens untested <INTENT.md> --map <file>` (#600, D56, D63, D178). The task's frozen inputs carry a JSON mapping file that lists each Windows-only intent line by its text with `golden:<case>`, `shadow` or null. A listed line with no cover is reported `untested` for the Linux runs; one JSON line is printed. A map entry that names no line of the intent, a repeated line or a bad field is refused, so a stale map fails closed. The intent is never marked and no line is recognised by its wording. No BBP change.
- **Known limitations:** which lines go in the map is experiment data that goes to Nano with the golden-set sign-off (Q4); nothing opens a claims task with it automatically yet.
