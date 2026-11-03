---
category: Added
changelog: remind-me files `tick:<uuid>` in a memory as a `task` reference, so `remind_me_references` finds everything about a rusty_tick task (ADR-0012, step 4).
---
## 2026-10-26 - remind-me: links to rusty_tick tasks (ADR-0012 step 4)

- **Added:** a memory that names a rusty_tick task as `tick:<uuid>` gets a `task` reference with the UUID (lowercased) as its value, so `remind_me_references` with kind `task` and that value lists every memory about the task. remind-me links to the task and never copies it.
- **Known limitations:** only the hyphenated UUID form is recognised. `tick:list:<uuid>` is a list, not a task, and is not filed as one.
