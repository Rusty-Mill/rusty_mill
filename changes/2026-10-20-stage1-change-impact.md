---
category: Added
changelog: Revision-bound proposed-change impact reports join Orch/CI dependency evidence, Millboard ownership and complete RemindMe requirements.
---
## 2026-10-20 - Stage 1 change impact

- **Added:** `rusty_orch inspect --impact request.json` identifies direct/transitive Cargo consumers and logical applications, includes binding requirements for inactive applications, records source/input/requirement revisions and emits required validation scope. Unknown consumers broaden validation.
- **Added:** Read-only Millboard ownership export and explicit complete RemindMe impact snapshots over its existing engine-backed exporter and optional retrieval. No new crate, graph engine or persistence format.
- **Fixed:** Send the bundled inspection program through stdin to avoid Windows process-launch error 206, retaining isolated Python imports, process deadlines and output bounds.
- **Known limitations:** Reports require a clean checkout and current frozen Cargo metadata cache. Refresh snapshots to detect live service changes. Durable requirements need explicit metadata scopes; prose is not automatically classified as binding. Inspection does not execute validation or change existing security/execution behavior.
