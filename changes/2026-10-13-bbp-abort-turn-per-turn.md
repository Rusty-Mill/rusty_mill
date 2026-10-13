---
category: Changed
changelog: **BBP per-turn abort.** `Command::AbortTurn` takes an optional turn id: `Some` ends that turn only, `None` ends every live turn; the host moderator aborts the turn whose harness died.
---
## 2026-10-13 - BBP per-turn abort

- **Changed:** `Command::AbortTurn` becomes `AbortTurn { turn: Option<TurnId> }`. With several live turns (`max_live_turns > 1`) a dead harness now forfeits only its own turn; the moderator passes the id, and at startup aborts each turn that predates it. `None` keeps the previous meaning (all live turns). An id that is not live is a no-op.
- **Known limitations:** the moderator still launches one harness at a time; one harness per live turn is the next change.
