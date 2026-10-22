---
category: Fixed
changelog: rusty_tick's .ics import matches tasks imported earlier even when their due date was edited since, so it no longer adds their events again.
---
## 2026-10-21 - rusty_tick: match earlier .ics imports after a due-date edit

- **Fixed:** a task imported from an .ics file before the key change (by bare UID, or by UID and position) was matched only while it kept its imported due date. Once its due date was edited, importing the file again added the event a second time. Such a task is now matched to the row that has the same title and is repeating or not in the same way, and the edit is kept.
- **Changed:** this match is made only when it is unambiguous: one row and one edited task, each with no other candidate. Look-alikes, such as two moved instances with the same title whose due dates were both edited, are imported anew instead of being guessed between.
- **Known limitations:** a task whose title was also edited, and the look-alikes above, are still imported again. So are tasks the calendar page imported before its import shared the importer's ids.
