---
category: Added
changelog: rusty_tick's web assistant snoozes tasks (“snooze dentist for 2 hours”, “snooze bins until friday at 9”).
---
## 2026-10-23 - rusty_tick: snooze in the web assistant

- **Added:** snooze: “snooze … for 30 minutes / 2 hours / a day / 1 week” and “snooze … until friday at 9” become `update_task` with `snooze`. Minutes and hours count from now (to the minute) as an instant, so a snooze through a DST fall-back hour lands on the right 01:15; days and weeks from the due day, or today for an overdue or undated task, keeping the time; “until” moves the task as “move … to …” does. Reminders count from the due time, so they move with it. An unreadable snooze changes nothing.
