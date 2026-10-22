---
category: Added
changelog: rusty_tick's web assistant lists open tasks by priority and/or tag, optionally due today, this week or overdue (“what's high priority today?”, “tasks tagged work”).
---
## 2026-10-22 - rusty_tick: priority and tag filters in the web assistant

- **Added:** “high priority”, “what's high priority today?”, “overdue low priority tasks”, “tasks tagged work”, “#errands”, “overdue tasks with tag "deep work"” and “high priority tasks tagged work due this week” become `filter_tasks {priority?, tag?, view}`, the view being all, today (overdue included, as in Today), next7 or overdue. A tag is “tagged X”, “tag X” or “#X”; quote one with spaces. A message is read as a filter only when every other word is filler (“show me my … tasks”) or names one view, so “add buy milk high priority” still adds a task, “tag milk with work” still tags one, and two views or two tags in one message are not guessed between.
- **Added:** `filter_tasks` is a browser tool over the UI's own views, with no MCP twin: `due_tasks` keeps the shape it shares with MCP, and an MCP agent filters `list_tasks` itself. A tag you do not have is named as such rather than answered with an empty list.
