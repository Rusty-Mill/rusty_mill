---
category: Added
changelog: **BBP records who answered each human gate.** `bbp human --as WHO` names the answerer in the op id, and `bbp metrics` lists every gate answer with it and a `self_approved` flag (true unless `--owner` names the answerer), so a scripted or sandbox run is labelled self-approved.
---
## 2026-10-15 - BBP gate answers in the run output

- **Added:** `metrics::gate_answers(events)` (each human approval or rejection with the op that carried it); `bbp human --as WHO` writes the op id as `by:WHO/<fresh>` (1 to 32 of letters, digits, `- _ .`); `bbp metrics [--owner ID]` prints `gates` (gate, `answered_by`, `self_approved`) and an overall `self_approved`.
- **Why the op id:** the log records only that the task's human principal acted, never who was behind it, and a protocol change is not warranted. The op is kept by the log and checked on replay, so the label is as durable as the log.
- **Rules:** with no `--owner`, every gate is self-approved (the safe default for a sandbox or scripted run); an unlabelled answer (no `--as`) is never the owner's; a run with no gate answered reports `self_approved: true`. The label is a claim by whoever ran the command, not an authenticated identity.
- **Not found:** the "card #29" this was asked against could not be located; the behavior follows the 2b criteria note.
