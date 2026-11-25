# #1092 Paste-ready prompt for Nano: the Mill Board inbox on BaileyAi

Status: 2026-11-25. Nano asked for the #1092 deploy (Let Em Loose thread, 2026-11-25 01:01Z: "Deploy for 1092"). Steps 1–3 of the layout's verification already passed in the smoke runs (`1166-smoke-run.md`). This prompt covers what was still missing: the `mill-board` user, the bbp guard, the sudoers rule and the systemd drop-in (`1092-deploy-layout.md` "Users", "systemd" and "Decision", D132), followed by verification steps 4 and 5. It makes no model call and costs $0.

The work is a committed script, `1092-deploy-board.sh`, so nothing is pasted into a terminal (see the `OB` note in `1092-deploy-prompt-bbp-and-board-dir.md`).

## Before you start (Nano)
- `/var/lib/mill-board/board.env` needs `MILL_BOARD_TICK_URL` and `RUSTY_TICK_TOKEN`. If `~/.mill-board/board.env` exists, the script carries it over, together with its `settings/`. If it doesn't, the script stops and asks you to run `sudoedit /var/lib/mill-board/board.env`. **Check this:** the layout assumes rusty_tick runs on BaileyAi (`http://127.0.0.1:8788`). If the cards live in the rusty_tick on ms-01, that URL points across the LAN and the tick token travels in plain HTTP. Decide whether that is acceptable before you fill it in.
- Step 4 needs an admin token for *this* board, set as `MB_TOKEN` in the Claude Code session's environment. Never paste it into the chat. A board created fresh writes one to `/var/lib/mill-board/settings/first-admin.token`, which you then delete.

## Prompt (paste into Claude Code on BaileyAi, started in `~/dev/rusty_mill`)

> Finish the Let Em Loose #1092 deploy on this host. Read `docs/projects/let-em-loose/1092-deploy-layout.md` and `1092-deploy-board-prompt.md` first. Don't change any design. If anything differs from the layout, stop and report.
>
> 1. `git fetch origin && git switch claude/project-thread-qf4lgi` (use `main` once that PR has merged). Then run `bash docs/projects/let-em-loose/1092-deploy.sh` if `/opt/lel/current` isn't this commit, and then `bash docs/projects/let-em-loose/1092-deploy-board.sh`. If it stops asking for `board.env` values, tell me and wait. Never print or cat `board.env`, and never show any token.
> 2. Verification step 4, against `http://127.0.0.1:8787` (or the board's `MILL_BOARD_HTTP`), with `Authorization: Bearer $MB_TOKEN`, as `POST /api/<op>` JSON calls:
>    a. `card_execution_link` with `{"id":"1092","execution":"board-check","goal":"board-check","bbp_task":"board-check","attempt":"1","assignment":"board-check","generation":1}`.
>    b. `inbox_list` `{}`. It must list the `board-check` A2 escalation (reason irreversible, card #1092). Cross-check it against `sudo -H -u orch-runner /opt/lel/current/bin/bbp inbox --dir /srv/lel/bbp --task board-check --json yes`.
>    c. `inbox_act` with that row's `id` and `rev`, `"action":"reject"` and `"dry_run":true`. Show me what BBP would write.
>    d. The same call without `dry_run`. Then `inbox_list` must no longer show it, and `bbp inbox ... --task board-check` must return `[]`.
> 3. Report the whole script output (no secrets), the four API results, and `sudo ausearch -m avc -ts recent`.

## Done when
Step 4 passes as above. The script has already shown steps 1 and 5. #1092's "Done when" is then met: the verification passed, and the docs PR is merged.
