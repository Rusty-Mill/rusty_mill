---
category: Added
changelog: rusty_tick web UI imports tasks from TickTick backups, Todoist CSV, any CSV with a Title column, .ics files and Apple Reminders (via an osascript export script).
---
## 2026-10-21 - rusty_tick: import from other apps

- **Added:** Settings → Account → Import from Other Apps. The file is recognised by its content: a TickTick backup CSV (lists, subtasks, checklists, tags, priority, status including Won't Do, repeat rules, reminders), a Todoist project CSV (list named after the file, `@labels` as tags, notes as comments, priority 1 as high), any CSV with a Title column (aliased List/Notes/Due/Start/Priority/Tags/Completed columns), or a `.ics` file. Missing lists are created by name; deeper subtasks hang off their top-level ancestor. Task and list ids are derived from the source, so re-importing adds nothing, even after an imported list is renamed, and finishes a done / won't-do write an earlier import lost, unless the task was edited since. Out-of-range dates (`2026-02-30`, `25:00`) are refused, not rolled over, and any date that cannot be read is kept in the task's notes.
- **Added:** `tools/export-apple-reminders.js`, a JXA script that writes Apple Reminders as that CSV, since Reminders has no export.
- **Known limitations:** created and completed times are not carried over; dates that cannot be read (e.g. Todoist's `every monday`) are kept in the notes rather than guessed; repeat rules outside the app's subset are dropped and counted; the Apple Reminders script has not been run on a Mac in CI and does not carry subtasks, tags or repeats.
