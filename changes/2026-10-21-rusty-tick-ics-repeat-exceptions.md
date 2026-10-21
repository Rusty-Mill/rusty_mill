---
category: Fixed
changelog: rusty_tick .ics import and calendar subscriptions key moved instances by RECURRENCE-ID and keep repeat exceptions (EXDATE, moved and cancelled instances); imports match tasks earlier imports created by due date.
---
## 2026-10-21 - rusty_tick: .ics instance identity and repeat exceptions

- **Fixed:** #829 keyed instances that share a UID by their order in the file, so adding or reordering instances gave one instance another's task and duplicated it. An instance's identity is now its UID plus its `RECURRENCE-ID` (none for the master).
- **Fixed:** repeat exceptions were discarded, so a moved instance showed beside the occurrence it replaced. `EXDATE`s (comma lists and repeated lines) and the occurrences that moved or cancelled instances replace become the master's `exDates`; an exception that cannot be read is counted in the import message, not guessed.
- **Fixed:** tasks imported by #829 (`[uid, n]`) or before it (bare UID) are still matched, but only to a row whose due date they still have, since their keys record a position rather than an instance. A master imported earlier and left untouched gets its exceptions on the next import; one the user has edited is left alone.
- **Fixed:** calendar subscriptions kept one task per UID, so a feed's moved instances were dropped, and showed the moved occurrence on its original day. Each moved instance now has its own task (stored under its UID and `RECURRENCE-ID`; masters keep the bare UID, so existing subscriptions match), and the feed owns the master's exceptions as it owns its dates and repeat rule.
- **Known limitations:** an earlier-imported task whose due date the user has since changed is not matched, so its event is imported again; #829 positions are searched up to the UID's count in the file plus 16.
