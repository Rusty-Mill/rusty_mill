---
category: Added
changelog: Mill Board decision inbox, slice 1: the Escalations port, the action-to-`bbp human` mapping and the BbpCli adapter (#595).
---
## 2026-11-03 - Mill Board decision inbox: port and bbp adapter

- **Added:** `rusty_mill_board::inbox`, the pure domain of the decision inbox (#595): the escalation shape `bbp inbox --json` prints, the four actions (Approve, Reject, Choose, Refine) and `plan`, which maps each action and escalation reason to the exact `bbp human` commands of the approved design.
- **Added:** `rusty_mill_board::bbp_cli::BbpCli`, the `Escalations` adapter over the host's `bbp` binary. Every command is fenced on the revision the escalation was read at; a multi-command action (Approve on a budget escalation: extend, then reopen) previews each command and fences the next on the previewed event count, so a concurrent write makes it `StaleRev` and the outcome says how far it got. Mill Board stores nothing and reads no BBP files.
- **Known limitations:** no MCP tools or web page yet (next slices). Refine is refused by the host, because claims mode does not yet accept a human `answer`. Approve and Refine on a task-total escalation are refused before anything is sent, because `bbp inbox` does not report which total was crossed.
