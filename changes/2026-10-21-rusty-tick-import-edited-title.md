---
category: Fixed
changelog: rusty_tick's .ics import also matches earlier-imported tasks whose title was edited along with their due date, and no longer takes an untouched task for a new instance.
---
## 2026-10-21 - rusty_tick: match earlier .ics imports after a title edit

- **Fixed:** a task imported from an .ics file before the key change, whose title and due date were both edited, was imported again. Once no row matches it by due date or by title, it now goes to the one row left for its UID that is repeating, or not, as it is. Pairing is still only made when it is unambiguous.
- **Fixed:** an earlier task nobody edited could be taken for a new instance of the same event when its own instance was gone from the file, so the new instance was never imported. Only a task edited since its import is matched this way now. A task at its creation version still has its imported due date, so a missing match means its row is gone.
- **Known limitations:** look-alikes (several instances of one event, all edited) are still imported again, as is a task whose repeat was also changed.
