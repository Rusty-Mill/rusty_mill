---
category: Added
changelog: docs/projects/mux/PREDICTIVE-ECHO.md, the design (not code) for rusty_mux predictive echo, for review before any implementation.
---
## 2026-10-29 - Predictive echo design note

- **Added:** `docs/projects/mux/PREDICTIVE-ECHO.md` designs predictive echo for C13 (Phase 3 step 3). It covers what the server can and cannot know about an echo (read order is exact; when the echo comes is a guess, including for the kernel's own echo, which a probe showed is not synchronous), a proposed `ECHO_ACK` feature with `EchoAck` and `EchoMode` messages, client epochs whose predictions are only drawn once one is confirmed, three layers of password suppression, composed and passing display, residual errors, a test plan and open questions.
- **Known limitations:** design only; no code, wire format or feature bit changes.
