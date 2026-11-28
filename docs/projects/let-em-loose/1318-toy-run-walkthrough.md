# #1318 toy run: step-by-step walkthrough for Nano

How to run and check the D167 toy-claim run for card #1318 (#600 slice 6, the Let Em Loose half of
#732). The run itself is `1318-toy-run.sh`; what it does step by step is in `1318-toy-run.md`.
This page is the order to do things in and what to look at. Decisions: D167 (one paid run), D188
(when a candidate is posted), D197 (runbook defaults), D199 (`builder sessions`).

**Where each step runs.** BaileyAi runs Fedora, so every command below is bash on BaileyAi, as
`baileyrd`, unless it says **PowerShell**. Windows is only used to ssh in.

**Checks versus host changes.** Most steps only read state. A few change the host for longer than
the run and are marked **host change**: installing binaries under `/opt/lel/<sha>`, installing
`claude-code-acp`, and turning on lingering. None of them is approved by this page or by the PR it
came in: each one needs your own separate OK at the time, and if you don't give it, stop there.

**What to share.** Never upload `~/toy-1318/out` whole or a raw log. When something needs a look,
send only the excerpt asked for, after checking it holds no account names, email addresses, tokens,
keys or private paths. Part F lists what the verdict needs.

**Deadline.** The Claude `no_overage` assertion lapses on 2026-12-25. Run before then. A new
assertion is only written after you re-verify, on the account itself, that it is the account `claude`
is logged in as and that its plan has no overage billing. Never change just the date to get past the
gate.

**Leftover sessions.** Before stopping a leftover `toy` tmux session or `lel-toy-1318-mux` unit,
confirm it belongs to an earlier unpaid pass:
```bash
ls -d ~/toy-1318/bbp 2>/dev/null; pgrep -af 'rusty_orch builder run'
```
If both print nothing, no paid pass was started, and stopping it is safe. If either prints
anything, a paid pass has started or is running. Don't stop anything: reattach with
`tmux attach -t toy` and let it finish (Part C).

## Part A. Get ready (free)

1. **PowerShell:** open a shell on BaileyAi (use your usual alias if you have one).
   ```powershell
   ssh baileyrd@192.168.10.104
   ```
2. Check that your systemd user manager outlives the ssh session. The run's mux server is a user
   unit, and if it stops mid-run both builders end `aborted`.
   ```bash
   loginctl show-user "$USER" -p Linger
   ```
   If it says `Linger=yes`, carry on. If it says `Linger=no`, **stop here**. Turning lingering on
   (`loginctl enable-linger`) is a persistent **host change**, so it waits for your separate OK.
   Without it, keep the ssh session and tmux open for the whole paid run.
3. Go to your checkout and put it on current `main` with no local changes (adjust the path if your
   clone lives elsewhere).
   ```bash
   cd ~/dev/rusty_mill && git fetch origin main && git switch --detach origin/main && git status --short
   ```
   The last command must print nothing.
4. Check `claude-code-acp` is installed for your user.
   ```bash
   command -v claude-code-acp && npm ls -g --depth=0 | grep claude-code-acp
   ```
   If nothing prints, **stop here**. Installing it (`npm install -g` of the pinned version you
   choose, D197 item 5) is a **host change** that waits for your separate OK. The script records
   the version but never installs it.
5. Check `claude` is logged in as the account in your assertion. This prints the account on your
   screen only; don't paste it anywhere. The script checks the assertion's date itself in step 2.
   ```bash
   claude auth status
   ```
6. **Pass 0, build.** Start the script once in tmux. On the first run for this commit it builds the
   five binaries, prints three `sudo install` lines and exits without touching anything else.
   ```bash
   tmux new -s toy 'bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'
   ```
7. The three printed `sudo` lines are a **host change**: they create `/opt/lel/<sha>/bin` (and
   leave `/opt/lel/current` alone). With your separate OK, run them; otherwise stop here.
8. Leave tmux: type `exit`.

## Part B. The free handshake (pass 1, no model call)

The script's step 4 starts `claude-code-acp` under `lel-jail` with the builders' roots and asks for
`initialize` and `session/new` only. No prompt is sent, so nothing is paid. It then asks you to type
`RUN`. On this pass you say no.

1. Start pass 1.
   ```bash
   tmux new -s toy 'bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'
   ```
   If tmux says the session already exists, check it as in "Leftover sessions" above. Only if it
   is from an unpaid pass, run `tmux kill-session -t toy` and start again.
2. Watch for these lines, in order:
   - `=== 2. assertion and login` then `claude: asserted and logged in`
   - `=== 3. mux server (lel-toy-1318-mux)`
   - `=== 4. jailed handshake (no prompt)`, then `initialize: ...`, `session/new: sessionId ...`
     and **`handshake ok`**
3. At `Type RUN to make the task and start the two paid builders`, type `no` and press Enter. It
   prints `STOP: not confirmed: nothing was made and nothing paid ran`.
4. Confirm the handshake passed and the mux unit stopped.
   ```bash
   grep -x 'handshake ok' ~/toy-1318/out/handshake.txt
   systemctl --user is-active lel-toy-1318-mux
   ```
   The first must print `handshake ok` and the second `inactive`. Pass 2 starts by clearing
   `~/toy-1318`, so note the result now; there is no need to keep or share the raw file.
5. Leave tmux: type `exit`.

**Pass:** `handshake ok` appeared and the unit is `inactive`. This also shows the first half of the
first done check: Claude's ACP adapter starts and answers under `lel-jail`.
**Fail:** see Part E. Nothing was paid, and you can rerun pass 1 as often as you like.

## Part C. The paid run (pass 2, the one D167 approved)

Two builders, one claim, a 45-minute wall each, through your subscription login. Plan for up to
about 50 minutes.

1. Start pass 2.
   ```bash
   tmux new -s toy 'bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'
   ```
2. Steps 0 to 4 repeat (all free). At the prompt, type `RUN` and press Enter.
3. Step 5 prints `claim <id>`. Note the number. Step 6 prints the two builder pids and starts a
   `running:` line every minute.
4. Open a second tmux window to watch (press `Ctrl-b` then `c`), then list the mux blocks.
   ```bash
   B=/opt/lel/$(git -C ~/dev/rusty_mill rev-parse --short=12 HEAD)/bin
   "$B/rusty-mux-ctl" --socket ~/toy-1318/mux.sock list
   ```
   You should see session `lel-toy-1318`, window `builders`, with blocks named
   `agent:toy-1318:<claim>:1` and `agent:toy-1318:<claim>:2`, each beside a viewer block.
5. Follow the rendered logs live.
   ```bash
   tail -F ~/toy-1318/state/builders/*/*/acp.log
   ```
   Press `Ctrl-c` to stop following (this only stops `tail`). Switch back to the script's window
   with `Ctrl-b` then `0`.
6. **Don't** press `Ctrl-c` in the script's window, stop or restart the mux unit, or start the
   script again while it runs. The builders are detached and keep going even if your ssh drops;
   reconnect with `tmux attach -t toy`.
7. When both builders end, step 7 reads everything back into `~/toy-1318/out` and the last line is
   either `LAUNCH PASS: ...` or `LAUNCH FAIL: ...`.

## Part D. Checking each done criterion

The card is done when all four hold.

| # | Done criterion | Where it's proved | Pass | Fail |
|---|---|---|---|---|
| 1 | A builder session for a toy claim runs in a mux block under `lel-jail` | Part B (handshake under `lel-jail`), Part C step 4 (the `agent:` blocks), and `LAUNCH PASS` | `handshake ok`; both `agent:toy-1318:<claim>:<n>` blocks listed; script ends `LAUNCH PASS` | No `agent:` blocks, or `LAUNCH FAIL` |
| 2 | A mux restart ends the turn as D104 says (it re-closed D20): `aborted`, nothing debited | CI, `builder-mux-restart` | Green in CI (row 4) | Red or missing in CI |
| 3 | Its `acp.log` renders | `~/toy-1318/out/builders/<claim>/<n>/acp.log` | Readable text lines: agent messages, tool calls with title and status, a usage line (or a note that usage wasn't reported) | Missing, empty, or raw JSON |
| 4 | The §11 tests pass in CI | Job "Orch BBP confined acceptance (Linux)" | Its summary lists `builder-stored`, `builder-mux-restart` and `builder-deadline` as passed | Any of the three failed or absent |

**Why the mux restart isn't done by hand.** Restarting the mux during the paid run would end a
builder `aborted`. That fails the launch-pass rule and spends the one approved run on a known
outcome. The behaviour is tested with a scripted fake agent instead: the `builder-mux-restart` case
in `rusty_orch`'s `builder/e2e.rs` kills and restarts a real mux server mid-turn and checks the row
ends `aborted`, nothing is stored or debited, the slot is free and the cgroup is empty.

**Reading the run's verdict (criteria 1 and 3).**
1. Show the result line and both session rows.
   ```bash
   tail -n 3 ~/toy-1318/out/script-*.log
   cat ~/toy-1318/out/sessions.jsonl
   ```
   Each row's `state` must be `stored` or `failed`, never `running` or `aborted`. `failed` still
   passes (D197): it means the builder didn't write `.lel/submit` or left an empty diff, so by D188
   nothing was posted. A `stored` candidate is reported, not required.
2. Open each rendered log.
   ```bash
   less ~/toy-1318/out/builders/*/1/acp.log
   less ~/toy-1318/out/builders/*/2/acp.log
   ```
3. If a candidate was stored, see it on the card.
   ```bash
   jq .current ~/toy-1318/out/card-after.json
   ```

**Checking CI (criteria 2 and 4).** Open
<https://github.com/Rusty-Mill/rusty_mill/actions/workflows/ci.yml?query=event%3Amerge_group> and
pick the latest run where the job "Orch BBP confined acceptance (Linux)" ran (it is skipped when a
merge touches no Linux-tested crate, so the newest run may not have it). The run's Summary page
lists each acceptance case. All three `builder-*` cases must show as passed. The verdict rule's
offline fixtures (`1318-toy-verdict.test.sh`) run in the "plan script unit tests" job on every run.

## Part E. If a step fails

**Before `RUN` (steps 0 to 4): nothing is paid.** Fix what the `STOP:` line names and start the pass
again. If it says the task dir exists, add `TOY_TASK=toy-1318b` in front of the command:
```bash
tmux new -s toy 'TOY_TASK=toy-1318b bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'
```
(The paths in Parts C and D then use `toy-1318b` in place of `toy-1318`.)

| `STOP:` says | Do this |
|---|---|
| `not on BaileyAi` | You're on the wrong machine. If BaileyAi's short hostname differs, put `LEL_HOST=<name>` in front of the command. |
| `<tool> is not on PATH` | Install that tool, or for `claude-code-acp` see Part A step 4. |
| `the checkout has local changes` / `HEAD ... is not on origin/main` | Repeat Part A step 3. |
| `Landlock is not active` | The kernel's LSM list lacks Landlock: `cat /sys/kernel/security/lsm`. Stop and tell Claude. |
| `no valid Claude no-overage assertion` | The assertion is missing or older than 30 days. Stop. Re-verify the account and its no-overage plan first; write a new assertion only if both still hold (see "Deadline"). Never just change the date. |
| `claude is not logged in as the asserted account` | Run `claude` and log in as that account, then check `claude auth status`. |
| `the systemd user manager has an ANTHROPIC_ variable` | `systemctl --user unset-environment ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN` |
| `lel-toy-1318-mux is already running` | Check it as in "Leftover sessions". Only if it is from an unpaid pass: `systemctl --user stop lel-toy-1318-mux` |
| `the mux server never bound` | Read `journalctl --user -u lel-toy-1318-mux -n 50` yourself. Send Claude only the error line(s), checked as in "What to share". |
| `... is not delegated to baileyrd` | Your user manager isn't giving the unit a delegated cgroup. Send Claude just the `STOP:` line. |
| `the jailed handshake failed` | Read `~/toy-1318/out/handshake.txt` yourself. If `lel-jail stderr` shows a refused write under `~/.claude`, **don't work around it**: a writable `~/.claude` root would change D166, so it comes back to you as a decision. Send Claude only the `STOP:` line and the one error line from `lel-jail stderr` or `jail status`, with any account or private path replaced by `<redacted>`. |
| `the claim was refused` / `bbp open was refused` / `rusty_orch builder sessions failed on the empty store` | Still unpaid. Send Claude the `STOP:` line and the error line above it, then retry with `TOY_TASK=toy-1318b`. |

**After `RUN` (step 6 started): don't rerun.** D167 approved one run. A `LAUNCH FAIL` keeps
everything, prints one `FAIL:` line per reason, and exits 1. Send the excerpts in Part F, plus the `FAIL:`
lines, and Claude works out what happened; it asks for any further excerpt by name. If your ssh drops, the builders keep going: reconnect and run
`tmux attach -t toy`.

## Part F. Send the results back

Send only these, as text pasted into the thread, after checking each as in "What to share":

1. The last three lines of the script log (the `LAUNCH PASS` or `LAUNCH FAIL` line and any `FAIL:`
   lines above it).
   ```bash
   tail -n 3 ~/toy-1318/out/script-*.log
   ```
2. The two persisted session rows and the two stdout rows.
   ```bash
   cat ~/toy-1318/out/sessions.jsonl ~/toy-1318/out/row-1.json ~/toy-1318/out/row-2.json
   ```
3. Your answer to criteria 1 and 3: whether you saw both `agent:` blocks, and whether both
   `acp.log`s read as text. Don't paste the logs themselves; Claude asks for a named excerpt if one
   is needed.

Everything else stays on BaileyAi under `~/toy-1318/out`. Claude then records the result in
`1318-toy-run.md` by PR, in remind-me and on card #1318 (D197 item 6). Whether #1320 goes ahead
stays your call (D167).
