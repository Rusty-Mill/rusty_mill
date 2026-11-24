---
category: Added
changelog: bbp human takes --decision ID and bbp decision reports whether an id was applied, so Mill Board settles an uncertain inbox send's Remediate card from BBP's log.
---
## 2026-11-24 - BBP decision ids settle uncertain inbox sends (#703)

- **Added:** `bbp human --decision ID` (1 to 64 of `[A-Za-z0-9._-]`, exclusive with `--op`) writes the op as `by:<as>/dec:ID`, so a repeat of the same send replays by R15 and the same id with another payload is `op_conflict`. `bbp decision --dir D --task T --decision ID[,ID2]` prints `[{"decision","applied","rev"}]` from the log; `rev` is the task revision just after the `op_accepted` that carries the id.
- **Added:** Mill Board sends a decision that drops acted-on work under its Remediate card's id (command `n` as `<id>.<n>`). A retry, a stale-rev retry, or a send refused with `StaleRev` now reads the lookup: applied keeps the card (with the rev noted), not applied closes it. Any other refusal, or a host without `bbp decision`, keeps the old note to check BBP's log.
- **Changed:** `Escalations::act` takes the optional decision id and the trait gains `applied`; `inbox_act`'s `recorded` answer carries `settled` (`applied`, `not_applied` or `unknown`).
- **Known limitations:** no new event variant and no schema change: the log already records every accepted human op, so old logs read as "not applied" and replay is unchanged.
