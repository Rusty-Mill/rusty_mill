# rusty_mill_runner

`mill-runner`: an optional process that works [Mill Board](..) cards through stages. Nothing in the
board depends on it. See [ADR-0002](../docs/adr/0002-optional-runner.md).

```text
mill-runner --stages stages.json --repo ~/src/app --work ~/mill-work \
            [--board http://127.0.0.1:8787] [--once] [--parallel 2] \
            [--read-root ~/.claude] [--env ANTHROPIC_API_KEY]
# board token: MILL_BOARD_TOKEN
```

`stages.json`:

```json
[
 {"name": "build", "from": ["ready", "building"], "enter": "building", "role": "builder",
  "argv": ["claude", "-p", "{prompt}"], "ok": "qa", "fail": "blocked", "network": true, "wall_secs": 1800},
 {"name": "test", "from": ["qa"], "role": "qa",
  "argv": ["cargo", "test"], "ok": "review", "fail": "building"}
]
```

- `argv` runs without a shell. Placeholders: `{prompt}` (card title, body, the latest notes),
  `{id}`, `{title}`, `{worktree}`. The command also sees `MILL_CARD_ID` and `MILL_STAGE`.
- Each card works in its own git worktree on branch `mill/<id8>`, under `rusty_sandbox`: writable
  worktree and scratch only, no local sockets of any kind (Unix or abstract) and no internet unless `network` (then IPv4/IPv6 only: loopback and LAN services are still reachable, see ADR-0002), `wall_secs` limit, clean environment
  (`PATH` and `--env` names only). The runner commits on success; the command never gets `.git`.
- **Running `claude -p` as a stage.** The `claude` binary is a Bun executable: besides its install
  directory it needs `--read-root /proc --read-root /dev/urandom` (without them it aborts in a few
  milliseconds). Give it `--permission-mode acceptEdits --allowedTools Read Write Edit Glob Grep` for a
  build stage, `"network": true`, and its API access through `--env ANTHROPIC_API_KEY` (or your gateway
  variables). Do not mount `~/.claude` or any token file just to make a login work: the sandboxed
  command could send it anywhere it can reach. `HOME` defaults to the card's scratch directory.
- Add `--read-root` for anything the command must read (toolchains, `~/.claude`), `--env` for variables
  it needs (visible to the command).
- A person moves cards into Ready and out of Review; the runner never merges. A holder of Triage can take
  a card back at any time. `max_attempts` (default 3) per stage, then Blocked.
- Startup refuses stages whose role lacks the board capability for the moves it makes.

Linux only (Landlock). Layout: `config` (stages) / `ports` (board, workspace traits) / `runner`
(stage loop, pure) / `http_board`, `git_work` (adapters).
