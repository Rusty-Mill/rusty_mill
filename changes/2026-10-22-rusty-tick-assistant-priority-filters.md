---
category: Added
changelog: rusty_tick's web assistant lists open tasks by priority, optionally due today, this week or overdue (“what's high priority today?”).
---
## 2026-10-22 - rusty_tick: priority filters in the web assistant

- **Added:** “high priority”, “what's high priority today?”, “overdue low priority tasks”, “medium priority this week” and “no priority tasks” become `priority_tasks {priority, view}`, the view being all, today (overdue included, as in Today), next7 or overdue. A message is read as a filter only when every other word is filler (“show me my … tasks”) or names one view, so “add buy milk high priority” still adds a task and two views in one message are not guessed between.
- **Added:** `priority_tasks` is a browser tool over the UI's own views, with no MCP twin: `due_tasks` keeps its shape shared with MCP, and an MCP agent filters `list_tasks` by priority itself.
