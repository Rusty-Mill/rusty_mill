# Issue tracker

GitHub Project board: https://github.com/orgs/Rusty-Mill/projects/1 (repo Rusty-Mill/rusty_mill). Proposed automation base branch: `staging` (inactive).
For current contributions, follow [CONTRIBUTING](../../CONTRIBUTING.md); this
proposal does not retarget PRs or change the merge workflow.

## Ticket format

```markdown
## Problem
<1–3 sentences: what is wrong or missing, from the user's side. No implementation plan.>

## Context
- **Where:** <page>, `<route>`
  - a. <step>
  - b. <step>
- **Who:** <who sees it>

![<page> · desktop](https://github.com/<owner>/<repo>/raw/<sha>/<path>.png)

## Fix
<the intended change in 1–3 lines>

## Acceptance Criteria
- [ ] <checkable outcome a test can assert>
- [ ] <checkable outcome>

## Risk
🟢 **Low** · <one line>

## Blocked by
- #<N> — <why>          ← or exactly: - None.
```

The removed `.claude/skills/super-board/references/ticket-format.md` is deferred
and unavailable in this checkout. The template above is standalone documentation;
it does not require that helper. See the [inactive proposal](../super-board/proposed-config.md).
