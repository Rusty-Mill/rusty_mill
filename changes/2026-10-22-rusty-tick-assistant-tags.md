---
category: Added
changelog: rusty_tick's web assistant adds and removes task tags (“tag dentist with health”, “remove tag health from dentist”).
---
## 2026-10-22 - rusty_tick: tags in the web assistant

- **Added:** “tag … with/as …” and “remove tag … from …”, “remove the … tag from …” or “untag … from …” become `update_task` with `tag` or `untag`. Several tags may be named (“#work, home and errands”). The split is at the last “with”, “as” or “from” outside quotes, so a quoted task name may hold one. Once the words say “tag”, a missing part gets a question, never a delete: “remove the milk” still deletes, “remove the work tag from” asks.
- **Added:** the browser adds a known tag by its name and a new one as said; the server creates it, as the tag picker's “create” does. Removing a tag the task lacks, a 21st tag, or a tag over 64 characters changes nothing.
