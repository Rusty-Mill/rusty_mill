---
category: Added
changelog: Let Em Loose #1092: script and prompt to deploy Mill Board as `mill-board` with the bbp guard on BaileyAi (D132).
---
## 2026-11-25 - Let Em Loose #1092: Mill Board inbox deploy script

- **Added:** `docs/projects/let-em-loose/1092-deploy-board.sh` installs the approved D132 layout (the `mill-board` user, the bbp guard and wrapper, the sudoers rule checked with `visudo -c`, and the systemd drop-in). Every check runs before any change, on every run: the host, and an existing unit, `board.env`, data and listener must be this loopback inbox board's. Step 4 acts only on `lel-1092-fixture`, a task the script opens and records (A2 only), after checking that the inbox row is exactly that fixture. A completed rerun is a no-op. It never prints `board.env` and makes no model call.
- **Added:** `1092-board-lib.sh` (the checks and the fixture flow) and `1092-board-lib.test.sh` (isolated fakes: a pre-existing unrelated task, a completed rerun, resume, an identity mismatch, a refused dry run, foreign units, env and listeners). The real reject is sent only after a validated dry run (accepted, with events) and must end `Done`; step 5 passes only on the guard's or sudo's own denial text.
- **Added:** `1092-deploy-board-prompt.md`, the paste-ready Claude Code prompt for step 4 (`card_execution_link`, `inbox_list`, `inbox_act` dry run, then reject).
- **Changed:** the BaileyAi board is a separate inbox board with its own SQLite file (`MILL_BOARD_DB`), because rusty_tick is retired (Nano, 2026-11-25; D175). The script runs verification step 4 itself, with the fresh board's admin token kept out of argv and output.
- **Changed:** CI's `plan script unit tests` job runs `1092-board-lib.test.sh` (bash and jq only).
- **Fail closed:** a failed `bbp open`, assignment or seed stops before anything else; a failed task lookup is not an absent task; every listener on the board's port must be the service on loopback, and a failed `ss`/`systemctl` inspection stops; the effective unit (`systemctl show`: fragment, every drop-in, user, environment files, ExecStart) is checked before any change and again before start.
- **Known limitations:** escalations show up on this second board, not on the live one. `-/run/sudo` is in `ReadWritePaths` up front (the layout's stated fallback).
