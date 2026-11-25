---
category: Added
changelog: Let Em Loose #1092: script and prompt to deploy Mill Board as `mill-board` with the bbp guard on BaileyAi (D132).
---
## 2026-11-25 - Let Em Loose #1092: Mill Board inbox deploy script

- **Added:** `docs/projects/let-em-loose/1092-deploy-board.sh` installs the approved D132 layout (the `mill-board` user, the bbp guard and wrapper, the sudoers rule checked with `visudo -c`, and the systemd drop-in). It then opens a toy `board-check` task whose only escalation is the A2 fixture. It is idempotent, never prints `board.env`, and makes no model call.
- **Added:** `1092-deploy-board-prompt.md`, the paste-ready Claude Code prompt for step 4 (`card_execution_link`, `inbox_list`, `inbox_act` dry run, then reject).
- **Changed:** the BaileyAi board is a separate inbox board with its own SQLite file (`MILL_BOARD_DB`), because rusty_tick is retired (Nano, 2026-11-25). The script runs verification step 4 itself, with the fresh board's admin token kept out of argv and output.
- **Known limitations:** escalations show up on this second board, not on the live one. `-/run/sudo` is in `ReadWritePaths` up front (the layout's stated fallback).
