---
category: Added
changelog: BBP specs can be co-edited by several live planners as line edits in the task log, with a section base check and a scripted load harness.
---
## 2026-10-25 - BBP co-edited specs

- **Added:** a `spec` version can be a `SpecEdit { brief, base, ops }`: `Insert { after, lines }` and `Delete { line }` ops on lines named by stable id, appended to the task log like any artifact. Each version has its own immutable text (`TaskState::doc`), so the plan gate still binds one exact text. Step 4a of `docs/bbp-event-based-plan.md`.
- **Added:** an edit anchored in a section that changed since its base is refused with `stale_base`, logged and not charged; an edit in an untouched section is applied to the head. `bad_edit` refuses an empty edit, an empty insert or a line with a line break.
- **Added:** the fold places inserted lines the way RGA does when ids follow the log, checked by a property test against a textbook RGA under random causal delivery. Accepting every edit (a CRDT's admission) therefore needs no new fold or crate.
- **Added:** `rusty_bbp/tests/coedit_load.rs`, scripted editors (no model, no spend) driving the real driver, engine and store at N = 2 to 32, and its report in the plan.
- **Known limitations:** no host or MCP surface yet, so live agents cannot write edits; the live N = 4 check needs its own approval. A build without `SpecEdit` refuses a log that has one. A snapshot spec cannot be an edit base.
