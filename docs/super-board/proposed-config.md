# Super-board configuration proposal — inactive

This is a documentation-only proposal, not an operational configuration. It was
moved out of `.claude/super-board/configs/rusty-mill.json` so locally installed
workers cannot discover it there. Do not copy it into a discovery path or run it.
No disabled-field semantics are assumed: inactivity comes from retaining this
example only as a fenced block in documentation, outside the operational path.

The values below preserve the proposal for review, including `staging`, automatic
merges and test/staging migrations. They are **not approved policy**. In particular,
`human_approves_merge: false`, `merge_policy.default: auto` and the migration
allowlist grant no authority. `cargo check --workspace` is only a proposed local
check, not a substitute for required CI, review or queue validation.

[CONTRIBUTING](../../CONTRIBUTING.md) remains the active workflow: reviewed,
green-CI PRs merged with a merge commit through the merge queue. Activation needs
a separately reviewed implementation addressing the removed tooling's findings
and an explicit bounded owner decision on autonomy, permissions and migrations.
This documentation does not approve local copies of the tooling.

```json
{
  "version": 1,
  "description": "rusty_mill build board",
  "project": {"owner": "Rusty-Mill", "title": "Rusty_mill board", "number": 1},
  "worker_backend": "workflow",
  "model_tier": "medium",
  "target": {"type": "repo", "url": null},
  "repo": {"path": ".", "remote": "https://github.com/Rusty-Mill/rusty_mill"},
  "base_branch": "staging",
  "timezone": "America/Chicago",
  "columns": ["Backlog", "Ready", "Building", "QA", "Review", "Blocked", "Done"],
  "labels": ["qa", "bug", "feature"],
  "paths": {"project_md": "docs/super-board/PROJECT.md", "pre_flight_md": "docs/super-board/pre-flight.md", "runs_dir": "docs/super-board/runs"},
  "human_approves_merge": false,
  "merge_policy": {"default": "auto", "auto_max_lines": 400, "allow_auto_on_production": false},
  "migrations": {"allowed_envs": ["test", "staging"], "target_env": "staging", "commands": {}, "checks": {}, "human_steps": []},
  "no_progress_cycles": 6,
  "truth_gate": "non-trivial",
  "truth_threshold": 70,
  "usage_pause_pct": 95,
  "usage_fallback": "none",
  "verify_commands": ["cargo check --workspace"],
  "max_workers": 0,
  "rebuild_cap": 2,
  "block_rate_alert_pct": 30,
  "collect": {"sources": ["github", "prs", "architecture"], "window_days": 14, "prs": {"max_prs": 200}, "architecture": {"max_findings": 5}, "custom": []},
  "notifications": {"channel": "session", "chat_id": null, "bot_identity": "baileyrd"}
}
```

See [project notes](PROJECT.md), [deferred pre-flight](pre-flight.md) and the
[standalone issue template](../agents/issue-tracker.md).
