# #1092 Paste-ready prompt for Nano: the Mill Board inbox on BaileyAi

Status: 2026-11-25 (separate inbox board, SQLite; review fixes: pre-flight before any change, script-owned fixture). Nano asked for the #1092 deploy (Let Em Loose thread, 2026-11-25 01:01Z: "Deploy for 1092"). Steps 1–3 of the layout's verification already passed in the smoke runs (`1166-smoke-run.md`). This prompt covers what was still missing: the `mill-board` user, the bbp guard, the sudoers rule and the systemd drop-in (`1092-deploy-layout.md` "Users", "systemd" and "Decision", D132, D175), followed by verification steps 4 and 5. It makes no model call and costs $0.

The work is a committed script, `1092-deploy-board.sh`, so nothing is pasted into a terminal (see the `OB` note in `1092-deploy-prompt-bbp-and-board-dir.md`).

## Before you start (Nano)
Nano chose a separate inbox board on BaileyAi with its own SQLite file (2026-11-25, "Second board on BaileyAi"). rusty_tick is retired, and the live board is untouched. The script needs no secrets from you. A fresh board writes its admin token to `/var/lib/mill-board/settings/first-admin.token`. The script uses that token for step 4 without printing it, and leaves the file for you to read and delete. On a rerun after you've deleted it, set `MB_TOKEN`.

## Run (on BaileyAi as baileyrd, from a checkout of this branch)
```
bash docs/projects/let-em-loose/1092-deploy.sh   # only if /opt/lel/current is not this commit
bash docs/projects/let-em-loose/1092-deploy-board.sh
```
Before it changes anything, the script checks that it is on BaileyAi (`hostname`; set `LEL_HOST` if the short name is not `baileyai`) and that any existing `mill-board.service`, `board.env`, `/var/lib/mill-board` and the process holding the board's port belong to this inbox board, on loopback. Anything else stops it. An existing `board.env` is checked, never edited. It installs files and restarts the service only when something changed.

Step 4 is part of the script. It acts only on `lel-1092-fixture`, a task it opens itself and records in `/var/lib/lel-deploy/1092-fixture.json` (root-owned). If a task by that name exists that it did not record, it stops. It adds a card on this board, links it to that task, calls `inbox_list`, and checks that the one row for the task is the recorded card and A2 claim. Only then does it call `inbox_act` (a dry run, then reject). It checks that both the board and `bbp inbox` are empty afterwards. Once it has verified the fixture, a rerun is a no-op. The `board-check` task from the first run is left as it is.

## Done when
The script ends with "Steps 1, 4 and 5 passed." #1092's "Done when" is then met: the verification passed, and the docs PR is merged.
