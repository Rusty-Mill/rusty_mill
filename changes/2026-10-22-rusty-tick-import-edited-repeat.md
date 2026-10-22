---
category: Fixed
changelog: rusty_tick's .ics import also matches earlier-imported tasks whose repeat was changed, and look-alike instances that were all edited, so their events are not added again.
---
## 2026-10-22 - rusty_tick: match earlier .ics imports after a repeat edit, and edited look-alikes

- **Fixed:** a task imported from an .ics file before the key change, whose repeat was changed (added or removed) as well as its due date, was imported again. After the title and repeat rounds, an edited task now goes to the one row left for its UID, so a repeat edit no longer hides it. As before, only tasks edited since import qualify, and a pair is made only when neither side has another candidate.
- **Fixed:** look-alike instances of one event (same title, all edited) were all imported again, since none could be told apart. When as many edited tasks as rows are left for them and no other row wants those tasks, each row now takes one. Which row takes which does not matter: every one is edited, so none is written, and nothing is added twice.
- **Known limitations:** when an event has more look-alike instances in the file than edited tasks left (a new instance among them), which one is new would be a guess, so they are imported again.
