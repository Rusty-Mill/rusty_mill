---
category: Added
changelog: rusty_bbp records human takeovers of agent sessions (takeover_started, takeover_input, takeover_ended) with a hash chain, and flags human-touched candidates.
---
## 2026-11-05 - rusty_bbp: human takeover events

- **Added:** claims-mode human takeover of an agent session (Let Em Loose D82, card #732).
  - The human `Takeover` action writes `takeover_started` before any input is possible, with one live takeover per session and at most 60 minutes.
  - The host records each delivered input batch as `takeover_input`: in order, at most 4096 bytes, each one chained by SHA-256 to the last.
  - `takeover_ended` closes the takeover with totals from the fold. Only the human can release one, and `expired` is accepted only after `until`.
  - `claim_card` gains `human_touched`: the current candidate was stored after a takeover on its claim in the same epoch (D84). Scoring is unchanged.
- **Known limitations:** core only. `bbp human takeover`, orch's input recording and the mux lease come in later slices. The events are added under claims schema 5 (D83), so a log that contains one can't be read by an older build.
