---
category: Fixed
changelog: A BBP harness that replaces its stdin keeps its agent slot after a moderator crash, and the moderator never signals a process group whose id may have been reused.
---
## 2026-10-21 - Slot holder for harnesses

- **Fixed:** an agent slot followed the harness's stdin. A launcher that replaced it (`exec </dev/null`) dropped the lock, so after a moderator crash the orphaned harness ran unadmitted beside the restarted moderator's next turn. The moderator now also hands the lock to `bbp slot-hold --group G`, a process in its own group that keeps the slot until the harness's process group is empty. The harness keeps the lock on its stdin as well.
- **Fixed:** the moderator checked whether a harness had exited by reaping it, then killed the harness's process group by its id. Once reaped, that id could already belong to an unrelated group. The moderator now checks without reaping (`waitid` with `WNOWAIT`), so the id stays reserved until the group has been killed.
- **Tests:** the moderator-crash regression also runs with a harness that redirects its stdin. With the holder disabled, that case fails.
- **Known limitations:** the holder starts just after the harness. If the moderator dies between the two launches and the launcher has already replaced the harness's stdin, nothing holds the slot for that harness. Closing this window needs a holder-owned launch or a startup handshake. If a harness group's id is freed and reused within the holder's 100 ms poll, the slot is held until the new group exits. Admission is late in that case, but never oversubscribed.
