---
category: Changed
changelog: Report-only erosion profiles are time-boxed; after 10 tasks or 14 days, new report-only claims tasks are refused until limits are approved.
---
## 2026-11-05 - Erosion gate: report-only pauses at expiry

- **Changed:** `bbp` records each report-only claims open in `<dir>/erosion/report-only.log` (one `<unix_ms> <task>` line, synced under a lock) before the open dispatches, and holds the ledger's lock until the open has published its task log, so racing opens can't overshoot. A ledger task counts only when its log starts with a claims open whose frozen profile set (checked against its digest) runs erosion report-only. Once 10 such tasks have opened, or 14 days have passed since the first line, a new report-only claims open is refused `bad_claims_config(erosion)`, saying that limits wait for Nano's approval from the calibration numbers (card #704, D42, D88). If the line can't be written, or the ledger is damaged, the open fails and nothing opens. Tasks already open, `"erosion": "off"` opt-outs and enforcing opens are unaffected.
- **Known limitations:** whether an enforcing open must match a human-recorded set of approved limits is open (OPEN_QUESTIONS Q20); until then enforcing opens work as before. The calibration report itself is slice 5.
