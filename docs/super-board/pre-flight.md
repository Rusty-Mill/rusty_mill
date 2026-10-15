# Super-board pre-flight — deferred

**Inactive proposal; not permission to run `super-board run`.** Skills, scripts,
workflows and guard hooks are present in `.claude/` but inactive: no config, permission
rules or auto-run hooks are committed. Use
[CONTRIBUTING](../../CONTRIBUTING.md) for the current contribution workflow.
The [proposed configuration](proposed-config.md) is documentation only.

The earlier 2026-10-14 lint notes (issue #685) reported a locally authenticated
`gh`, Rust 1.98.1, Node, Python with `PYTHONUTF8=1` on Windows, an origin `staging`
branch and `rusty_tick` workspace membership. Those are historical observations,
not verified checkout readiness, credential requirements or activation approval.
Do not grant token scopes, change credentials or run migrations from this list.

Before any future activation, an independent review must confirm the tooling's
validation and fail-closed behavior (the findings from the first review are fixed in the
tooling PR, with tests, but not yet independently re-reviewed). The owner must
explicitly decide the bounded autonomy policy, including any permissions, merge
or migration authority. None of those decisions is made by this documentation PR;
local tooling copies are not certified by it.
