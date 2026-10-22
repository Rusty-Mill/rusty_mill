---
category: Fixed
changelog: rusty_tick's .ics import also matches an earlier-imported task whose repeat was changed along with its due date, so its event is not added again.
---
## 2026-10-22 - rusty_tick: match earlier .ics imports after a repeat edit

- **Fixed:** a task imported from an .ics file before the key change, whose repeat was changed (added or removed) as well as its due date, was imported again. After the title and repeat rounds, an edited task now goes to the one row left for its UID, so a repeat edit no longer hides it. As before, only tasks edited since import qualify, and a pair is made only when neither side has another candidate.
- **Known limitations:** several edited instances of one event that are left over after every round are still imported again rather than guessed between.
