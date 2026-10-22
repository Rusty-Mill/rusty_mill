---
category: Added
changelog: BBP event-based plan: replace the moderator poll, per-turn harnesses and turn-order scheduling with events, and place `rusty_crdt` as an optional artifact fold measured at realistic agent counts.
---
## 2026-10-22 - BBP event-based plan

- **Added:** `docs/bbp-event-based-plan.md`, the owner-requested plan to make BBP event-based instead of loop-based, in four steps: a push-based moderator, event-driven scheduling as the default, long-lived agents woken by a `wait` tool, and several live agents on shared artifacts. Only the last step can need a CRDT; artifact edits become ops in the log so `rusty_crdt` can slot in as a second fold, and the need is measured with scripted editors at 2 to 32 agents instead of a two-agent probe.
- **Known limitations:** docs only; every step needs its own owner approval. The 2b evidence run tested loop mode and does not answer the CRDT question.
