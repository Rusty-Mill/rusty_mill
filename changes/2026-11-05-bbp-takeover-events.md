---
category: Added
changelog: rusty_bbp records human takeovers of agent sessions (takeover_started, takeover_input, takeover_closing, takeover_ended) with a hash chain, and flags human-touched candidates.
---
## 2026-11-05 - rusty_bbp: human takeover events

- **Added:** claims-mode human takeover of an agent session (Let Em Loose D82, card #732).
  - The human `Takeover` action writes `takeover_started` before any input is possible, with one live takeover per session and at most 60 minutes.
  - The host records each delivered input batch as `takeover_input`: the raw bytes as the mux delivered them (base64 in the log, so invalid or split UTF-8 survives), in order, at most 4096 bytes, each one chained by SHA-256 to the last. A batch is accepted up to 10 s after `until`, so one delivered just before expiry can still be posted.
  - The human's release, or closing or cancelling the task, writes `takeover_closing`: input stops at once, and the host may still record batches already delivered for 10 s. The host then writes the one `takeover_ended` with totals from the fold, for the reason it closed with. A Closed or Cancelled task takes those two host writes for a closing takeover and nothing else.
  - The host also ends a live takeover itself (`expired`, only after `until`; `revoked`, `turn_ended`, `mux_lost`), but never as `released`.
  - `claim_card` gains `human_touched`: the current candidate was stored after a takeover on its claim in the same epoch (D84). Scoring is unchanged.
- **Known limitations:** core only. `bbp human takeover`, orch's input recording and the mux lease come in later slices. The events are added under claims schema 5 (D83), so a log that contains one can't be read by an older build.
