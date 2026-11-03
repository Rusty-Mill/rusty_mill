---
category: Added
changelog: BBP claims mode accepts the human's Refine answer (C-R10), and bbp inbox rows carry the brief, acted-on claims with their marks, and crossed task totals with one allowance (#595).
---
## 2026-11-03 - BBP inbox follow-up: Refine, task totals, acted-on

- **Added:** C-R10. In claims mode the human channel may post an `answer`, which is the human's clarification of the intent (the inbox's Refine). It must ref the task's brief, so #598 can migrate it into an intent version. Its `reply_to` is a claim on the task, or absent for the whole task. It needs no turn, is not charged, changes no claim, and is accepted while the task is open or escalated. An agent's `answer` is still `wrong_mode`. Trace CL-81.
- **Added:** `bbp human refine MSG|task BRIEF TEXT` builds that answer.
- **Added:** `bbp inbox` rows gain three fields:
  - `brief`: the task's brief.
  - `acted`: the shown claim and candidates that were acted on, with the references of their marks that were not aborted. This is metadata only: the host creates no card.
  - `totals`: for a task-total row, each exhausted field with its spent amount, its limit and one allowance, which is the limit the task opened with.
- **Changed:** `rusty_bbp::engine::exhausted_fields` is now public.
- **Known limitations:** this change is host metadata and the `refine` verb only. Mill Board's planner in #1413 to #1417 does not use them yet, so Refine and task-total decisions stay unsupported there until the board wiring in #1422 (Refine through `refine` with the brief, task-total Approve as one allowance, and the Remediate card). No diffstat of the acted-on candidate is reported yet.
