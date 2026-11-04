---
category: Changed
changelog: bbp human extend-claim refuses on an evidence run, as extend does.
---
## 2026-11-04 - bbp: no claim budget extensions in evidence runs

- **Changed:** `bbp human extend-claim` (and the fenced and `--dry-run` forms) refuses on a task that carries an evidence marker, with the same `BadHumanInput` rejection and admission lock as `extend`, and appends nothing. On any other task it still applies. Owner decision, 2026-11-04: it gives an evidence run more budget, so the no-extensions rule (#1366) covers it.
- **Changed:** a capped first start of `bbp mod` also refuses a task whose log already holds a claim budget extension.
