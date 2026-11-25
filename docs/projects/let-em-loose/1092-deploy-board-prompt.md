# #1092 Paste-ready prompt for Nano: the Mill Board inbox on BaileyAi

Status: 2026-11-25 (separate inbox board, SQLite). Nano asked for the #1092 deploy (Let Em Loose thread, 2026-11-25 01:01Z: "Deploy for 1092"). Steps 1–3 of the layout's verification already passed in the smoke runs (`1166-smoke-run.md`). This prompt covers what was still missing: the `mill-board` user, the bbp guard, the sudoers rule and the systemd drop-in (`1092-deploy-layout.md` "Users", "systemd" and "Decision", D132), followed by verification steps 4 and 5. It makes no model call and costs $0.

The work is a committed script, `1092-deploy-board.sh`, so nothing is pasted into a terminal (see the `OB` note in `1092-deploy-prompt-bbp-and-board-dir.md`).

## Before you start (Nano)
Nano chose a separate inbox board on BaileyAi with its own SQLite file (2026-11-25, "Second board on BaileyAi"). rusty_tick is retired, and the live board is untouched. The script needs no secrets from you. A fresh board writes its admin token to `/var/lib/mill-board/settings/first-admin.token`. The script uses that token for step 4 without printing it, and leaves the file for you to read and delete. On a rerun after you've deleted it, set `MB_TOKEN`.

## Run (on BaileyAi as baileyrd, from a checkout of this branch)
```
bash docs/projects/let-em-loose/1092-deploy.sh   # only if /opt/lel/current is not this commit
bash docs/projects/let-em-loose/1092-deploy-board.sh
```
Step 4 is part of the script. It adds a card on this board, links it to the `board-check` task, calls `inbox_list`, and then calls `inbox_act` (a dry run, then reject). It checks that both the board and `bbp inbox` are empty afterwards.

## Done when
The script ends with "Steps 1, 4 and 5 passed." #1092's "Done when" is then met: the verification passed, and the docs PR is merged.
