---
category: Changed
changelog: The Let Em Loose inbox fixture claim A2 is worded as a proposed action that has not run, so the lenses stop citing it as a test run.
---
## 2026-11-18 - Let Em Loose fixture A2 wording

- **Changed:** in `docs/projects/let-em-loose/1166-seed-inbox.sh` the fixture claim A2 now reads "Proposed action, not run: run `sweep --yes` on the toy data directory with DAYS 7." It used to read "Run `sweep --yes` on the toy data directory with DAYS 7, after the dry-run list was reviewed.", which the lenses took for a test report (they cited msg 3 as a run in smoke-3, smoke-4 and smoke-6) and which asserted a dry-run review that never happened. Nano's Q61 answer. Seed claim A keeps its original wording as the control. Thresholds, roster and escalation are untouched.
- **Known limitations:** this does not close Q53 (invented or mis-cited evidence); it removes one trigger. Tested on a throwaway `bbp` store (A2 escalates at 12 as before); not run on BaileyAi.
