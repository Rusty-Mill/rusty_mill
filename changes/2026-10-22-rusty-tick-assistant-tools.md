---
category: Added
changelog: rusty_tick's web assistant can find tasks, list what's due or overdue, complete, reopen, rename, reschedule, reprioritize, move between lists and delete tasks, and list your lists, using the MCP server's tool names.
---
## 2026-10-22 - rusty_tick: the MCP tools in the web assistant

- **Added:** besides adding tasks, the assistant understands:
  - “find …” (`search_tasks`)
  - “what's due today?”, “due this week” and “what's overdue?” (`due_tasks`)
  - “complete …”, “reopen …” and “delete …” (`complete_task`, `reopen_task`, `delete_task`)
  - “rename … to …”, “move/reschedule/postpone … to …”, and “make/mark/set … high/medium/low/no priority” or “set priority of … to …”, and “move … to list …”, “move … to the … list” or “move … to inbox” (`update_task`)
  - “lists” (`list_lists`)

  Each command is a frontend tool call that the browser runs against its store, named as the MCP server's tool. The server's tool table uses the same name constants, so the two cannot drift.
- **Added:** a task is named by its title: an exact title first, else the only title containing the words. When several match, the assistant lists them and asks; it never guesses. Completing considers open tasks only and reopening done ones only; the Trash is never searched.
- **Changed:** the assistant's add tool is now `add_task` (it was `create_task`), the MCP name. “What's due?” now reads every list through `due_tasks`, not only the view that is open, so the panel no longer sends the open view as context.
- **Added:** “rename … to …” and “move … to …” never guess where the name ends. A quoted name or new title is taken whole, “to” inside it included (“rename "buy milk" to "talk to mum"”). An unquoted rename needs exactly one “ to ”; with more, the assistant asks for quotes. A move cuts at the last “ to ”, since a day never contains one (“move talk to mum to friday”). The browser reads the day in your own time zone: today, tomorrow, a weekday (the next one, or a week on if it is today), in N days, next week, YYYY-MM-DD, or none to clear the dates. A timed task keeps its time of day, an undated one becomes all-day, and a start that would fall after the new due date is dropped. A day it can't read, or a blank title, changes nothing.
- **Added:** a priority phrase ends in a fixed word pair (“… high priority”), so the name before it can't be cut wrongly. In “set priority of … to …” the cut is at the last “ to ”, since a priority never contains one. Priority words are high, medium (or med), low, and none (or no); any other word changes nothing.
- **Added:** a move to a list is told from a move to a day by its list words, since a day never ends in “list”. With both list forms present, the last is the cut, so a title may contain “to list” too. The browser names the list as it names a task: an exact name first, else the only live list containing the words. An unknown, archived, blank or ambiguous list changes nothing.
- **Fixed:** the command words are matched case-insensitively in ASCII only, so a character whose lower case is longer (such as the Kelvin sign) can't shift where a name is cut.
- **Notes:** the assistant is still rule-based, not a model.
