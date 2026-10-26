---
category: Added
changelog: **`rusty_remind_me`** adopts ADR-0028, one client-neutral contract for every agent that uses remind-me (design only).
---
## 2026-10-26 - remind-me: one contract for every client (accepted)

- **Added:** ADR-0028 (Accepted 2026-10-26), a design for serving Claude, Hermes bots, local models and other LLMs from one contract: a registered client id per caller, per-client tool profiles with a five-tool `minimal` tier, CI-checked tool rules, one usage guide served by the server, a capture path per client and a `conformance` command. No code changes.
- **Known limitations:** the Hermes, local-model and multi-model rows of the client inventory are partly assumed until `conformance` confirms them.
