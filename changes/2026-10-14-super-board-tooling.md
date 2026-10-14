---
category: Added
changelog: **super-board tooling.** The board pipeline's scripts, workflow, hooks and skills, with the 12 review findings from #708 fixed and regression tests.
---
## 2026-10-14 - super-board tooling

- **Added:** `.claude/` bin, hooks, skills, workflows and settings for the super-board pipeline, scripts and hooks committed executable.
- **Fixed:** merge gate uses the repo's allowed merge method, checks the PR's real target branch, and merges only on green CI for the exact head; card moves never replay an uncertain write; assignees are kept; the planner reads the whole blocked-by history; Build hands its branch and PR to QA; the QA skill no longer creates or resets accounts; three hooks (cleanup, delete guard, push guard) close the holes found in review.
- **Added:** `.claude/tests/`, 62 tests, run with `python3 -m unittest discover -s .claude/tests`. They are not wired into CI.
- **Known limitations:** file modes were checked in the git index only, not on a fresh POSIX checkout. The pipeline stays unproven end to end until a card runs through it.
