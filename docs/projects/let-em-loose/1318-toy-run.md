# #1318 Paste-ready prompt for Nano: the D167 toy-claim run on BaileyAi

Status: 2026-11-26, ready to run from `main` once this runbook and the read-only `rusty_orch builder sessions` command it uses (awaiting your approval) are merged. All of #1318's PRs are merged: #1880, #1882, #1885, #1886, #1887, #1889 (S5) and #1890 (S6). The script checks for S6. This is the one paid run D167 approved: two builders, a 45-minute wall each, one claim. Claude calls go through your subscription login (D165). The defaults are in D197. The no-overage assertion you added on 2026-11-25 lapses on 2026-12-25, so the run has to happen before then, or you re-date the assertion first.

The work is a committed script, `1318-toy-run.sh`, plus a small helper, `1318-acp-handshake.py`. Nothing gets pasted into a terminal.

## Before you start (Nano)
- **`claude-code-acp` installed for your user.** Use `npm install -g @zed-industries/claude-code-acp`, and pin the version you want. This is your own install, not a repository dependency (D197 item 5). The script records the version but never installs it.
- **`claude` logged in** as the account in your assertion.
- **A clean checkout of `main`** that includes all six slices.
- **No sudo during the run.** The script uses sudo for nothing. The first time, it builds the binaries and prints three `sudo install` lines for `/opt/lel/<sha>/bin`. Run them, then run the script again. `/opt/lel/current`, `/srv/lel` and the inbox board stay untouched.

## Run (on BaileyAi as baileyrd)
```
tmux new -s toy 'bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'
```
Use tmux so that a dropped terminal can't stop the read-back. The builders themselves are detached and keep running either way.

## What it does
0. **Read-only pre-flight.** It checks that it's on BaileyAi, not running as root, the tools are present, and the checkout is clean and on `origin/main` with S6 merged. It also checks that Landlock is active and the task is unused.
1. **Binaries.** It builds the five binaries (`bbp`, `rusty_orch`, `lel-jail`, `rusty-mux-server`, `rusty-mux-ctl`), or uses them if they're already in `/opt/lel/<sha>/bin`.
2. **Fail-closed checks.** `rusty_orch lens check` must report Claude "ready if logged in as <account>". That means your assertion is present and fewer than 30 days old. `claude auth status` must name that account. Either failure stops the run before anything is made.
3. **Its own mux server.** It starts a mux server in a delegated systemd user unit, `lel-toy-1318-mux`. That lets the builders' cgroups be made below it without root. Every STOP before step 6 stops this unit again.
4. **A no-cost handshake.** `claude-code-acp` starts under `lel-jail` with the builders' roots and answers `initialize` and `session/new`. No prompt is sent, so no model call is made. If the agent can't start in the jail, for example because it needs a write under `~/.claude`, the run stops here. Once every free check has passed, it asks you to type `RUN`. Anything else stops it with nothing made and nothing paid.
5. **Toy repo and claim.** It creates a fresh toy repo in `~/toy-1318/repo` (a stub `sweep` and one usage test), plus a claims task with its own BBP dir. As `architect` it posts the one claim: build `sweep --dry-run DIR DAYS` with tests. Then it checks that `rusty_orch builder sessions` reads the run's empty store. If it can't, the run stops before anything is paid, and a retry needs `TOY_TASK=toy-1318b`.
6. **The paid part.** It launches two `rusty_orch builder run`s at once on that claim. Each runs one Claude turn under a 45-minute wall. A builder's diff is posted only if it writes `.lel/submit` and the diff isn't empty (D188). You can watch the builders in the mux: session `lel-toy-1318`, window `builders`.
7. **Read-back.** It saves both stdout session rows, the persisted sessions (`sessions.jsonl`), both `acp.log`s, `bbp card`, `bbp why --claim` and `bbp verify` in `~/toy-1318/out`, checks the launch-pass rule, and stops the mux unit. The pass rule trusts the store, not stdout: a builder prints its row even when the final save failed. It needs exactly two persisted sessions for the claim, with different `n`, each `stored` or `failed` with the state and candidate its stdout row reports, and no `save:` note. A persisted `failed` session still passes (D197). Anything unreadable, missing or different is a `LAUNCH FAIL`, and the script exits 1 after keeping everything. The rule is `toy_verdict` in `1318-toy-verdict.sh`; `bash 1318-toy-verdict.test.sh` runs its offline fixtures.

## Done when
The script prints `LAUNCH PASS`:
- both rows ended `stored` or `failed` by D188's rule, none `running` or `aborted`;
- each has an `acp.log`;
- every session cgroup is empty;
- both agent slots are free.

A stored candidate is reported but not required (D197 item 6). Then send Claude `~/toy-1318/out`. Whether #1320 goes ahead stays your call (D167).

## If it stops
A STOP before step 6 makes no paid call. Fix what it names and run the script again (use `TOY_TASK=toy-1318b` if it asks): the task dir is only made in step 5. After step 6 has started, don't rerun: D167 approved one run. Send the output dir instead.

## Result
_Filled in by PR after the run: date, commit, both rows, tokens, whether a candidate was stored, anything the read-back flagged._
