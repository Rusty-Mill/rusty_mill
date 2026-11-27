---
category: Added
changelog: `rusty_orch lens why` prints a claim's `bbp why` history with the lens job behind each evidence post (#599 §5, D216).
---
## 2026-11-27 - `rusty_orch lens why`

- **Added:** `rusty_orch lens why --bbp --dir --task --claim --state [--at]` prints one claim's `bbp why --claim` history as one JSON line. Each evidence step that a saved blind review posted gains `job: {lens, epoch, look_harder, unsure}`. Read-only.
- **Docs:** Let Em Loose D216 to D218 and Q67. The `rusty_orch lessons` part of #599 is stopped: remind-me's HTTP add route doesn't carry `project` yet (D58's gate).
- **Known limitations:** model and cost aren't reported, because `lens_record` stores neither (Q67).
