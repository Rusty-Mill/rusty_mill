---
category: Fixed
changelog: rusty_tick import no longer loses a done/won't-do status after a failed write, drops duplicate rows with suffix-looking titles, or invents deadlines from out-of-range dates.
---
## 2026-10-21 - rusty_tick: import data-loss fixes

- **Fixed:** a done / won't-do write lost between creating a task and closing it was skipped forever on re-import. The next import now finishes it, conditional on the task's creation etag, so a task edited since is left alone.
- **Fixed:** rows with the same title (`Task`, `Task`, `Task#2`) could share a key, and one was silently dropped. Keys now encode their parts and occurrence unambiguously. Tasks imported under #821's keys are still matched (each claimed once, the first row to produce a colliding old key owns it), so re-importing those files skips, resumes or adds the row #821 lost, without duplicates or overwriting edits.
- **Fixed:** out-of-range dates (`2026-02-30`, `2026-13-01`, `25:00`, offsets past ±14:00) were rolled over into real deadlines. They are now refused, and any unreadable date is kept in the task's notes.
- **Known limitations:** the Apple Reminders export script is still untested on macOS.
