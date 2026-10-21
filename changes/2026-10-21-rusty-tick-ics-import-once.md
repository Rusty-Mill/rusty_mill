---
category: Fixed
changelog: rusty_tick adds an .ics file's tasks once, from the calendar page or Import from Other Apps, and keeps repeat exceptions (EXDATE, moved and cancelled instances) so the calendar shows each occurrence where the source has it.
---
## 2026-10-21 - rusty_tick: one .ics import path

- **Fixed:** the calendar page's .ics import created tasks with fresh ids, so importing the same file again, or through Settings → Account → Import from Other Apps, duplicated every event. It now uses the importer's reader and writer, so tasks get ids derived from each event's UID and a repeat import adds only what is new.
- **Fixed:** the importer keyed .ics rows by bare UID, so the moved instances of a repeating event (which share its UID) were silently dropped. An instance's identity is now its UID plus its `RECURRENCE-ID` (none for the master), so it keeps its task when instances are added or the file is reordered; the bare UID is kept as the legacy key, so files imported before still match.
- **Fixed:** repeat exceptions were discarded, so a moved instance showed beside the occurrence it replaced. `EXDATE`s and the occurrences that moved or cancelled instances replace now become the master's `exDates`; an exception that cannot be read is counted in the import message. A master imported earlier and left untouched gets its exceptions on the next import; one the user has edited is left alone.
- **Known limitations:** tasks the calendar page imported before this change have random ids and are not matched; importing such a file again adds its events once more.
