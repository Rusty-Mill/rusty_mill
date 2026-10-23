---
category: Added
changelog: rusty_tick can merge a duplicate task into the one to keep (task menu, "Merge into…"); an import never brings the merged duplicate back.
---
## 2026-10-22 - rusty_tick: merge duplicate tasks

- **Added:** a task row's menu has "Merge into…", which opens a picker. It lists the other live tasks with the same title, and the search box reaches any task by title. The task picked is kept as it is. The duplicate goes to the Trash, so a merge can be undone by restoring it.
- **Added:** a merge records the duplicate's id as an alias of the kept task, in a new `import_alias` document kind that the server accepts. An import derives each task's id from its source, so before this it would recreate a merged duplicate once the duplicate was purged from the Trash. `applyImport` now writes to the kept task instead. Aliases are re-pointed on later merges, so they never chain or loop.
- **Added:** the kept task is never written by an import. A task at its creation version that a row reaches through an alias is the user's untouched task, not one whose status or exception write was lost, so no status or repeat exceptions are copied onto it. The picker never offers the duplicate's own subtasks: trashing the duplicate trashes them too.
- **Fixed:** an .ics row's fallback match no longer takes an edited task that another row already resolves to, so a merged task is not also handed to a different instance (which would have dropped that instance).
- **Notes:** the action is in list view's row menu. Aliases are not included in backups, like subscriptions and saved filters.
