# Super-board settings

Permission rules are **not committed**. Under the accepted
[autonomy policy](autonomy-policy.md) (decision 4) each person who runs the board adds the
narrow set below to their own `.claude/settings.local.json` (git-ignored by Claude Code).
Adding it is a personal permission change, made by you, not by this repository.

What the committed `.claude/settings.json` keeps: the guard hooks only. They can block a
command or edit; they never allow one.

## Narrow set (policy decision 4)

The original 41 rules, minus the wildcards `gh api`, `git push`, `gh pr merge`, `gh project`
and `gh pr edit`, plus pushes only to `claude/*` branches. The merge gate's own `gh pr merge`
runs inside the script, so it needs no rule of its own. `gh api` is left out: lanes should use
`super-board-card.sh` and the REST helpers. If a lane stalls on a prompt, add the one command
it needs, not the wildcard.

```json
{
  "permissions": {
    "allow": [
      "Bash(gh issue view:*)",
      "Bash(gh issue edit:*)",
      "Bash(gh issue comment:*)",
      "Bash(gh pr view:*)",
      "Bash(gh pr diff:*)",
      "Bash(gh pr checks:*)",
      "Bash(gh pr comment:*)",
      "Bash(gh pr create:*)",
      "Bash(gh pr ready:*)",
      "Bash(gh project item-edit:*)",
      "Bash(gh project item-list:*)",
      "Bash(git worktree:*)",
      "Bash(.claude/bin/super-board-card.sh:*)",
      "Bash(git checkout:*)",
      "Bash(git add:*)",
      "Bash(git commit:*)",
      "Bash(git pull:*)",
      "Bash(git fetch:*)",
      "Bash(git blame:*)",
      "Bash(mkdir:*)",
      "Bash(pgrep:*)",
      "Bash(node --check:*)",
      "Bash(bash .claude/bin/super-board-wave-plan.sh:*)",
      "Bash(bash .claude/bin/super-board-preflight.sh:*)",
      "Bash(bash .claude/bin/super-board-usage.sh:*)",
      "Bash(bash .claude/bin/super-board-throttle.sh:*)",
      "Bash(bash .claude/bin/super-board-codex-lane.sh:*)",
      "Bash(bash .claude/bin/super-board-codex-wave.sh:*)",
      "Bash(bash .claude/bin/super-board-host.sh:*)",
      "Bash(bash .claude/bin/super-board-pr-body.sh:*)",
      "Bash(bash .claude/bin/super-board-merge-gate.sh:*)",
      "Bash(bash .claude/bin/super-board-env-check.sh:*)",
      "Bash(cargo test:*)",
      "Bash(cargo check:*)",
      "Bash(cargo clippy:*)",
      "Bash(cargo fmt:*)",
      "Bash(git push -u origin claude/:*)",
      "Bash(git push origin claude/:*)"
    ]
  }
}
```

## Original 41 rules (reference only, do not use)

Auto-approves these commands for the agent, including `git push`, `gh api` and `gh pr merge`.
`gh pr merge` is reached only through the merge gate, which refuses a wrong target, a
disallowed merge method and red or missing CI for the head, but the allow rule itself is
broad. The owner decides whether any of these is acceptable, and for which lanes.

```json
{
  "permissions": {
    "allow": [
      "Bash(gh project:*)",
      "Bash(gh issue view:*)",
      "Bash(gh issue edit:*)",
      "Bash(gh issue comment:*)",
      "Bash(gh pr view:*)",
      "Bash(gh pr diff:*)",
      "Bash(gh pr checks:*)",
      "Bash(gh pr comment:*)",
      "Bash(gh pr create:*)",
      "Bash(gh pr ready:*)",
      "Bash(gh project item-edit:*)",
      "Bash(gh project item-list:*)",
      "Bash(gh api:*)",
      "Bash(git worktree:*)",
      "Bash(.claude/bin/super-board-card.sh:*)",
      "Bash(git checkout:*)",
      "Bash(git add:*)",
      "Bash(git commit:*)",
      "Bash(git push:*)",
      "Bash(git pull:*)",
      "Bash(git fetch:*)",
      "Bash(git blame:*)",
      "Bash(mkdir:*)",
      "Bash(pgrep:*)",
      "Bash(node --check:*)",
      "Bash(bash .claude/bin/super-board-wave-plan.sh:*)",
      "Bash(bash .claude/bin/super-board-preflight.sh:*)",
      "Bash(bash .claude/bin/super-board-usage.sh:*)",
      "Bash(bash .claude/bin/super-board-throttle.sh:*)",
      "Bash(bash .claude/bin/super-board-codex-lane.sh:*)",
      "Bash(bash .claude/bin/super-board-codex-wave.sh:*)",
      "Bash(bash .claude/bin/super-board-host.sh:*)",
      "Bash(bash .claude/bin/super-board-pr-body.sh:*)",
      "Bash(gh pr edit:*)",
      "Bash(bash .claude/bin/super-board-merge-gate.sh:*)",
      "Bash(gh pr merge:*)",
      "Bash(bash .claude/bin/super-board-env-check.sh:*)",
      "Bash(cargo test:*)",
      "Bash(cargo check:*)",
      "Bash(cargo clippy:*)",
      "Bash(cargo fmt:*)"
    ]
  }
}
```

## Session-start hook (not used)

Deletes branches and worktrees that look merged, automatically, on every session start,
after a `git fetch`. The cleanup logic has tests, but the hook acts without asking, so it
is not part of the accepted policy and is not committed.

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cd \"$CLAUDE_PROJECT_DIR\" && python3 .claude/hooks/cleanup-wt.py --auto --fetch 2>/dev/null || true"
          }
        ]
      }
    ]
  }
}
```
