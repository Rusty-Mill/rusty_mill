# Issue tracker

GitHub Project board: https://github.com/orgs/Rusty-Mill/projects/1 (repo Rusty-Mill/rusty_mill). Base branch: `staging`.

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

Full rules: .claude/skills/super-board/references/ticket-format.md
