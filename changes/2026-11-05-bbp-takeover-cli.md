---
category: Added
changelog: bbp gains `human takeover`/`takeover-release` and the host commands `takeover-input` and `takeover-ended` for recording agent-session takeovers.
---
## 2026-11-05 - bbp: takeover commands

- **Added:** the host side of human takeover of an agent session (Let Em Loose D82, card #732).
  - `bbp human takeover MSG SESSION CLIENT MINUTES REASON…` and `bbp human takeover-release N`.
  - For orch: `bbp takeover-input --takeover N --seq S --sha256 HEX`, with the batch's raw bytes on stdin (any bytes, at most 4096, read without buffering more), and `bbp takeover-ended --takeover N --reason expired|revoked|turn-ended|mux-lost`.
- **Known limitations:** orch doesn't call these yet. That comes with the mux input gate (Rusty Term) and the launch slice.
