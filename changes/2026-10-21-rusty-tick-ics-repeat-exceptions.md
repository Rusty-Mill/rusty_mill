---
category: Fixed
changelog: rusty_tick .ics import keys moved instances by RECURRENCE-ID and keeps repeat exceptions (EXDATE, moved and cancelled instances), matching tasks earlier imports created by due date.
---
## 2026-10-21 - rusty_tick: .ics instance identity and repeat exceptions

- **Fixed:** #829 keyed instances that share a UID by their order in the file, so adding or reordering instances gave one instance another's task and duplicated it. An instance's identity is now its UID plus its `RECURRENCE-ID` (none for the master).
- **Fixed:** repeat exceptions were discarded, so a moved instance showed beside the occurrence it replaced. `EXDATE`s (comma lists and repeated lines) and the occurrences that moved or cancelled instances replace become the master's `exDates`; an exception that cannot be read is counted in the import message, not guessed.
- **Fixed:** tasks imported by #829 (`[uid, n]`) or before it (bare UID) are still matched, but only to a row whose due date they still have, since their keys record a position rather than an instance. A master imported earlier and left untouched gets its exceptions on the next import; one the user has edited is left alone.
- **Known limitations:** an earlier-imported task whose due date the user has since changed is not matched, so its event is imported again; #829 positions are searched up to the UID's count in the file plus 16.
