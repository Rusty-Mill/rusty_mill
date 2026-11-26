---
category: Added
changelog: rusty_orch: #600 builder sessions: the Builder job kind with its admission check, the `builders` config block and the orch-store `builder_session` table (#1404).
---
## 2026-11-25 - rusty_orch: builder session admission and table (#1404)

- **Added:** `orch_lens::builder`, the #600 builder job kind (D66, D177). Admission refuses a session, naming the limit, when the claim is not open, has a live candidate, has `builders_per_claim` live sessions, the board has `max_live_builders` live, or `spent < budget` fails (the existing budget refusal read ahead). `recover` ends every live session as aborted after a restart, debiting nothing.
- **Added:** the lens config's optional `builders` block: `builders_per_claim` (default 1), `max_live_builders` (3) and `builder_wall_secs` (2700), each above 0.
- **Added:** orch-store's `builder_session` table (`builder-sessions.mmap`, schema tag `BuilderSessionRecord::v1`), one record per BBP board: claim, number, candidate, state, start and end, tokens in and out, and the run's per-profile results and erosion delta. Rows are checked on save and load. No existing record changes.
- **Changed:** the approved #600 design is back in `docs/projects/let-em-loose/600-builder-session-design.md` (Q44); `600-build-plan.md` sections 2 to 5 and 10 now follow it (D177, D178).
- **Known limitations:** nothing calls admission yet; launching a builder (worktree, jail, mux) is #1318. The Windows-only line check is #1405.
