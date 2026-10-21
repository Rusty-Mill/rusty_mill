---
category: Security
changelog: Orch's Millboard token never reaches a child process, and a card-linked state cannot be resumed without its card.
---
## 2026-10-21 - Board token stays in the Orch process; linked state needs its card

- **Security:** `orch-cli` removes the parent's own credentials (`PARENT_SECRETS`, now `RUSTY_ORCH_BOARD_TOKEN`) from every child it launches, and so does the BBP supervisor. Moderators, harnesses and provider CLIs never inherit the board token.
- **Fixed:** resuming a card-linked state without `--card`/`--board` is refused before a new generation is saved or any host command runs, so the card never falls behind a generation that acted.
