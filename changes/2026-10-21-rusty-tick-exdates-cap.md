---
category: Fixed
changelog: rusty_tick caps a task's repeat exceptions at the server's 1000, keeping the latest, so a calendar subscription or .ics import with more no longer fails.
---
## 2026-10-21 - rusty_tick: cap repeat exceptions at 1000

- **Fixed:** a feed event with more than 1000 exceptions (`EXDATE`s and moved or cancelled instances) made every refresh of its subscription fail, since the server refuses more than 1000 `exDates` on a task. Subscriptions now store at most 1000, as the .ics import already did.
- **Changed:** past the cap the latest exceptions are kept, not the earliest, for both subscriptions and import: a long-running series' recent and future exceptions are the ones the calendar shows. One helper (`storedExDates`) does this for both.
- **Changed:** the browser demo's in-memory store refuses more than 1000 `exDates` too, as the server does; the shared API contract test now covers the limit against both.
- **Known limitations:** exceptions older than the latest 1000 are dropped, so those old occurrences can show again on the calendar.
