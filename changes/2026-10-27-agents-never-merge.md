---
category: Changed
changelog: "Docs: agents, including the super-board robot, no longer queue or merge PRs; DottiusMaximus manages reviews and merges."
---
## 2026-10-27 - Agents never queue or merge PRs

- **Changed:** `AGENTS.md` and `CONTRIBUTING.md` now say agents open PRs and request review from DottiusMaximus, who manages reviews and merges (owner decision, 2026-10-27). They previously told the super-board robot to merge its own PRs and exempted those PRs from review.
- **Changed:** `docs/super-board/autonomy-policy.md` marks its decision 1 ("the robot merges") as superseded.
- **Changed:** `.claude/super-board/configs/rusty-mill.json` sets `merge_policy.default` to `"human"`, so the merge gate leaves every robot PR for a person instead of merging it.
- **Known limitations:** the gate still merges when someone runs it by hand; the rule is enforced by policy, not by a branch ruleset.
