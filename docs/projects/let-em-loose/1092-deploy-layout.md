# #1092 Deploy layout: Let Em Loose v0 on BaileyAi (M1)

Status: design for Nano's approval (card #1092 step 1). Nothing is deployed.
Authority: D53 (one host), D130 (design batch-approved), D131 (deploy on BaileyAi authorised), D94 (work on Nano's machines goes as a paste-ready prompt), D127 (no new design). Paid model calls, schema/API changes and other ask-first items still go to Nano.

## Binaries
`cargo build --release -p rusty_bbp_host -p rusty_orch -p rusty_mill_board` gives `bbp`, `codex-bbp-harness`, `claude-bbp-harness`, `rusty_orch`, `rusty_mill_board`.
Install to `/opt/lel/<git-sha>/bin/`, symlink `/opt/lel/current -> <git-sha>`. Put `/opt/lel/current/bin` on the PATH of orch-runner's moderator (agents.json and the pool launch `bbp` and harnesses by name).

## Users
- `orch-runner`: runs `rusty_orch`, `bbp mod`, harnesses; owns BBP dir, orch state, rootless podman `lel-codex`.
- `baileyrd`: Nano's login; reads logs and the UI.
- Mux: M1 deploys none. #600 builder sessions come after the first run (D129). When it lands it runs as `orch-runner` (mux-hosted-agents-design.md section 7), not `baileyrd` as the card says; the design wins.
- `mill-board`: system user (no login) for the Mill Board service (D132); owns `/var/lib/mill-board` 0700 (`board.env` 0600). Not in the `orch-runner` group. See "Decision".

## Directories (outside any source checkout and workload read roots)
| Path | Owner / mode | Notes |
|---|---|---|
| `/srv/lel/bbp` | orch-runner 0700 | `--dir` / `BBP_DIR`; holds `master` |
| `/srv/lel/work` | orch-runner 0700 | runner `--work`; runner refuses if others can write |
| `/srv/lel/orch-state` | orch-runner | `--state` / `RUSTY_ORCH_STATE` |
| `/srv/lel/slots` | orch-runner | optional `slots.dir` |
| `/srv/lel/{profiles,bbp,lens}.json` | orch-runner 0640 | profile set frozen at `bbp open` |
| `/etc/let-em-loose/assertions.json` | root:orch-runner 0640 | read by `lens check`; only Nano writes it (`no_overage`, `host_cleared`, 30-day expiry) |
| `~baileyrd/lel/machine-id` | existing, 444 | R5; ro bind-mount at `/etc/machine-id` in `lel-codex`; keep on recreate |
| `/var/lib/mill-board` | mill-board 0700 | board state and `board.env` 0600; replaces `~baileyrd/.mill-board` |
| `/usr/local/libexec/mill-board-bbp` | root 0755 | `exec sudo -n -u orch-runner /opt/lel/current/bin/bbp "$@"` |

`/etc/sudoers.d/mill-board` (root 0440): `mill-board ALL=(orch-runner) NOPASSWD: /opt/lel/current/bin/bbp inbox --dir /srv/lel/bbp *, /opt/lel/current/bin/bbp human --dir /srv/lel/bbp *`

## systemd
- `mill-board.service`: the shipped unit plus a drop-in: `User=mill-board`, `EnvironmentFile=/var/lib/mill-board/board.env`, `ReadWritePaths=/var/lib/mill-board /srv/lel/bbp` (the `bbp human` child writes the log), `NoNewPrivileges=false` (sudo), `MILL_BOARD_BBP_BIN=/usr/local/libexec/mill-board-bbp`, `MILL_BOARD_BBP_ROOT=/srv/lel/bbp`. rusty_tick must already be up.
- No `lel-orch.service`: `rusty_orch run` runs to completion, it is not a daemon. M1 starts runs by hand as orch-runner. A unit would be new design (D127).

## Network (plain HTTP, loopback)
- Board to tick: `MILL_BOARD_TICK_URL=http://127.0.0.1:8788` + `RUSTY_TICK_TOKEN`. Board listens `127.0.0.1:8787` (`MILL_BOARD_HTTP`); set `MILL_BOARD_ALLOWED_HOSTS`.
- orch to board (only with `--card`): `--board http://127.0.0.1:8787`, env `RUSTY_ORCH_BOARD_TOKEN/ACTOR/ROLE` (role with Build; its own agent token, created once in Settings).
- orch to llama-swap: `base_url` in the lens config (`openai-http`; refuses non-loopback/non-RFC1918). Port unverified: 8080 appears only in tests; confirm on the host.
- Claude/Codex: subscription logins only, no API keys in env. Codex only via `podman exec lel-codex`; Claude host-side as orch-runner.
- The board must run where `bbp` runs (#595 I3).

## Upgrades
Drain, then flip: (1) no open tasks (`bbp inbox`; `rusty_orch run --cancel <reason>` for leftovers); (2) build to `/opt/lel/<new-sha>`; (3) `ln -sfn`; (4) `systemctl restart mill-board`; (5) run verification; (6) roll back by repointing `current`. Never flip mid-task (running tasks launch `bbp mcp` by name).

## Verification (as orch-runner unless stated; no paid calls)
1. `curl -fsS http://127.0.0.1:8787/health` returns `{"ok":true}`.
2. `rusty_orch lens check /srv/lel/lens.json --assertions /etc/let-em-loose/assertions.json --repo <repo>` exits 0. An unavailable provider is a finding, not a deploy failure.
3. `bbp open` a toy claims task on a throwaway repo (pattern: `crates/apps/rusty_bbp_host/docs/proving-run.md`, `BBP_DIR=/srv/lel/bbp`); `bbp card` and `bbp why` print JSON.
4. Link the task from a card; `inbox_list` shows its escalations (cross-check `bbp inbox --dir /srv/lel/bbp --task <T>`); `inbox_act` on a toy escalation proves the board can write the log.
5. As mill-board, under the unit (`systemd-run -p User=mill-board ...` with the drop-in's settings): `/usr/local/libexec/mill-board-bbp inbox --dir /srv/lel/bbp --task <t> --json yes` succeeds, and `sudo -n -u orch-runner /bin/cat /srv/lel/bbp/master` is refused. If sudo fails because `ProtectSystem=strict` leaves `/run/sudo` read-only, add `/run/sudo` to `ReadWritePaths`.

## Decision: which user runs Mill Board? (D132, Nano 2026-11-16T19:17Z)
The inbox runs `bbp inbox` and `bbp human`, which need `master` (owner-only, `orch-runner`). Mill Board runs as its own user `mill-board`, not `orch-runner`, so the host-side Claude harness cannot read the board's tokens. It calls `bbp` as `orch-runner` through the wrapper and a sudoers rule limited to those two verbs on `/srv/lel/bbp`. The cost is the unit's `NoNewPrivileges=true`. Rejected: running as `orch-runner` (Nano's first choice, reversed); group-sharing `/srv/lel/bbp` with `master` 0640 (hands the board the master secret and needs group-writable log files).

## Drift found
- Card says mux runs as `baileyrd`; design says `orch-runner`.
- Mill Board README says the board never writes BBP files; in fact `bbp human` does.
- `rusty_orch` README "Getting started" omits `lens` and `--bbp`; `args.rs` USAGE is correct.
- bbp README paths assume `/tmp/bbp` and user `nano`; do not reuse under `/srv`.
- Breaker is not wired into `lens run` (#758); meter and ceilings are.
