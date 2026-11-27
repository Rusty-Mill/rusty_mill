---
category: Added
changelog: Mill Board anchors each linked BBP task's claim-log head as an append-only card comment, outside the account that writes the log (#1551).
---
## 2026-11-27 - Anchor the BBP claim-log head outside the log writer's account (#1551)

- **Added:** `rusty_mill_board` anchor pass (`anchor.rs`, `Board::anchor_tick`). With `MILL_BOARD_BBP_ANCHOR_SECS` set, the board, running on the same host as its own user, runs `bbp verify --expect <last anchor>` for each task a card links or already anchors and records the head as a comment by `bbp-anchor:mill-board`, a name no credential can act as. A rewrite by `orch-runner` of the log up to the last anchor, including any legacy prefix the chain covers, is recorded once as `FAILED`. Design in `docs/projects/let-em-loose/1551-claim-log-anchor-design.md` (D213-D215, D224). The deploy guard admits read-only `bbp verify --task T [--expect H]`. No BBP log format or API change.
- **Known limitations:** off by default. Installing the guard line and setting the timer on BaileyAi need Nano's separate host-change authorisation. Root on the host is not covered: an off-machine copy is deferred (D215) until outside or untrusted agents join. A log made only of legacy unchained lines is not anchored.
