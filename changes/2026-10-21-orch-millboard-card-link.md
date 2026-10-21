---
category: Added
changelog: Hosted Orch runs record their assignment generation with the Millboard card they work for, and stop when the card has moved past it.
---
## 2026-10-21 - Orch records hosted generations with Millboard cards

- **Added:** `rusty_orch run --card <id> --board <url>` binds the goal's execution to a Millboard card. After the run takes over its assignment, it records that generation with the card before any host command: `card_execution_link` the first time, `card_execution_recover` (compare-and-set) afterwards. A card already at a later generation stops the run before it touches the task.
- **Added:** a plain-HTTP Millboard client in `rusty_orch` on `rusty_http`'s blocking transport, with no new registry dependency. The token comes from `RUSTY_ORCH_BOARD_TOKEN` and is kept out of debug output.
- **Security:** `orch-cli` removes the parent's own credentials (`PARENT_SECRETS`, now the board token) from every child it launches, as does the BBP supervisor, so moderators, harnesses and provider CLIs never inherit the board token.
- **Changed:** resuming a card-linked state without `--card`/`--board` is refused before a new generation is saved or any host command runs, so the card never falls behind a generation that acted.
- **Added:** required acceptance mode `millboard-link`, which runs the real Orch, BBP and Millboard binaries through take-over, a stale state copy, and recovery.
- **Known limitations:** `https://` boards are not supported. Millboard still accepts `card_execution_recover` from any Build holder, so the board relies on its callers to be Orch. The link's attempt is always `1`. Terminal states are not yet reported to the card.
