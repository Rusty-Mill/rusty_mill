---
category: Added
changelog: rusty_tick's web assistant can find tasks, list what's due or overdue, complete, reopen and delete tasks, and list your lists, using the MCP server's tool names.
---
## 2026-10-22 - rusty_tick: the MCP tools in the web assistant

- **Added:** besides adding tasks, the assistant understands:
  - “find …” (`search_tasks`)
  - “what's due today?”, “due this week” and “what's overdue?” (`due_tasks`)
  - “complete …”, “reopen …” and “delete …” (`complete_task`, `reopen_task`, `delete_task`)
  - “lists” (`list_lists`)

  Each command is a frontend tool call that the browser runs against its store, named as the MCP server's tool. The server's tool table uses the same name constants, so the two cannot drift.
- **Added:** a task is named by its title: an exact title first, else the only title containing the words. When several match, the assistant lists them and asks; it never guesses. Completing considers open tasks only and reopening done ones only; the Trash is never searched.
- **Changed:** the assistant's add tool is now `add_task` (it was `create_task`), the MCP name. “What's due?” now reads every list through `due_tasks`, not only the view that is open, so the panel no longer sends the open view as context.
- **Notes:** the assistant is still rule-based, not a model. `update_task` stays MCP-only, since “move X to Friday” is too loose to parse reliably.
