---
category: Added
changelog: Let Em Loose #1166 smoke run seeds a fixture claim that escalates to the inbox (D146), so the inbox is exercised even when the lenses drop the action claim.
---
## 2026-11-17 - Let Em Loose #1166 inbox fixture

- **Added:** `docs/projects/let-em-loose/1166-seed-inbox.sh`: posts an irreversible claim A2 and two fixture supports (principals `seed-s1`, `seed-s2`) so it reaches the strict line (12) and escalates `irreversible` before any model call. `1166-smoke-run.sh` runs it after seeding and its roster declares the two principals. Thresholds and escalation rules are untouched.
- **Added:** `1166-smoke-rerun.sh`: the rerun of #1166 as one script (fresh task `smoke-2`, the inbox fixture, a detached lens run, read-back). It refuses to start unless the installed `rusty_orch` has the D144 parse fix and no closed-pipe panic, and says to run `1092-deploy.sh` from `main` first. `1166-smoke-run.sh` now runs `lens run` detached (setsid), so a closed terminal cannot stop it.
- **Known limitations:** A2's escalation is a fixture, not a lens finding; the lenses' own handling of A is still judged by the run. Verified on a throwaway `bbp` store (escalate, human drop, verify clean), not on BaileyAi.
