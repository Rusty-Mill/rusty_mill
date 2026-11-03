---
category: Changed
changelog: "Agents, including the super-board robot, no longer queue or merge PRs: DottiusMaximus manages reviews and merges, and a new `merge_policy.handoff_to` makes the board gate hand PRs off instead of merging them."
---
## 2026-10-27 - Agents never queue or merge PRs

- **Changed:** `AGENTS.md` and `CONTRIBUTING.md` now say agents open PRs and request review from DottiusMaximus, who manages reviews and merges (owner decision, 2026-10-27). They previously told the super-board robot to merge its own PRs and exempted those PRs from review. `docs/super-board/autonomy-policy.md` marks its decision 1 ("the robot merges") as superseded.
- **Added (super-board):** `merge_policy.handoff_to: "<login>"`. When set, `super-board-merge-gate.sh` exits 13 with `handoff: @<login> reviews and merges PR #<N>` before reading or requesting any approval, so even a valid pinned `done` cannot reach `gh pr merge`. The Reviewer prompts (wave workflow, Codex lane, `run.md`, `super-review`) request review from that login and park the card in Blocked without offering `done`. Boards without the key behave as before.
- **Changed:** `.claude/super-board/configs/rusty-mill.json` sets `handoff_to: "dottiusmaximus"` (and `default: "human"` as a fallback if the key is ever removed).
- **Known limitations:** a person running `gh pr merge` by hand is outside the gate; the rule there is policy, not a ruleset.
