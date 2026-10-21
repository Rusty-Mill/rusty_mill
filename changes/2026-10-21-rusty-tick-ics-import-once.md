---
category: Fixed
changelog: rusty_tick adds an .ics file's tasks once, whether it is imported again from the calendar page or through Import from Other Apps, and keeps moved instances of repeating events.
---
## 2026-10-21 - rusty_tick: one .ics import path

- **Fixed:** the calendar page's .ics import created tasks with fresh ids, so importing the same file again, or through Settings → Account → Import from Other Apps, duplicated every event. It now uses the importer's reader and writer, so tasks get ids derived from each event's UID and a repeat import adds only what is new.
- **Fixed:** the importer keyed .ics rows by bare UID, so the moved instances of a repeating event (which share its UID) were silently dropped. Keys now add the occurrence; the bare UID is kept as the legacy key, so files imported before still match.
- **Known limitations:** tasks the calendar page imported before this change have random ids and are not matched; importing such a file again adds its events once more.
