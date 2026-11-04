---
category: Added
changelog: BBP claims mode gains intent versions (`intent_revised`) and a host task pause (`task_paused`) for cost ceilings and the breaker (#598).
---
## 2026-11-04 - BBP intent versions and host task pause (#598)

- **Added:** the human action and event `intent_revised { brief, affects }` (C-R11). It stores the new brief as the task's current one and bumps the intent version, a fold that starts at 1. Each affected claim that is accepted reopens in a new epoch (`claim_reopened.by: intent`), or escalates `intent_changed` if it was acted on. `affects` is every claim, or the union of named claims and domains; an empty or unknown one is refused `bad_affects`. Claim cards show the current version and the one each claim was accepted under, once the intent has been revised. CLI: `bbp human … intent FILE [claims=msg:1,msg:2] [domains=a,b]`.
- **Added:** the host action and event `task_paused { reason, detail }` (C-R12), with `spend_ceiling`, `call_ceiling` or `breaker`. It moves an open task to `escalated` until the human resumes it open. CLI: `bbp pause --reason R --op OP [--detail TEXT]`, which replays on the same op.
- **Added:** both inboxes show the new reasons. `bbp inbox` reports `TaskPaused` with the pause, and `IntentChanged`. Mill Board's inbox approves a pause by resuming open and an intent change by accepting the claim.
- **Known limitations:** a ceiling pause recurs on resume unless the ceiling was raised in the Factory config. The orch lens runner doesn't write `bbp pause` yet; that lands with the breaker. Nothing new is written until used, so existing logs stay byte-identical.
