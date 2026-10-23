---
category: Added
changelog: rusty_tick's web assistant lists today's habits, checks them in (or clears today's check-ins) and adds daily habits.
---
## 2026-10-22 - rusty_tick: habits in the web assistant

- **Added:** “habits” (`list_habits`) lists the habits due today with today's check-ins against each goal. “check in …” (`check_in_habit`) adds one check-in today, up to the goal; at the goal it says so rather than clearing the day as one more click on the page would. “undo check in …” clears today's check-ins. “add habit …” (`add_habit`) adds a once-a-day habit, refusing a name already taken.
- **Added:** a habit is named as a task is: its exact name, else the only live name containing the words; when several match, it asks. Habits are client documents with no API route, so these tools run only in the browser and have no MCP twin. The panel loads the habits as the habits page does.
- **Changed:** the habits store gains `setCount(habitId, day, count)`; a click's `toggle` is now built on it.
