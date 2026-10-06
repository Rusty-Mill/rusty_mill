# rusty_bot

Per-bot sandboxes for AG-UI agents. A bot is an agent process (anything
that serves `POST /api/agent`: `rusty_tick`, `echo_agent`, a Python
server) run confined by [`rusty_sandbox`](../rusty_sandbox): its own
workspace as the only writable directory, an explicit list of readable
roots, a complete environment, hard limits, and a process group that dies
as one. The network stays open, because a bot listens on a port and may
call a model; the filesystem is what keeps one bot's data from another's.

Step 7 of the [ADR-0007 follow-ons](../../../docs/dev_phases/ADR_0007/FOLLOW-ONS.md), second PR.

## Shape

| Piece | What it does |
| --- | --- |
| `BotSpec` | Name, program, args, environment, workspace, read roots, limits. `sandbox()` is the `SandboxSpec`: write only to the workspace, read the roots and the program's own directory, start in the workspace with exactly that environment. |
| `default_limits()` | A day of CPU, a week of wall clock (a bot is stopped by its process group, not the clock), 2 GiB of address space, 1 GiB files, 1024 descriptors, 256 processes. |
| `load(json)` | Bots from a JSON array of `{"name", "program", "workspace", "args"?, "env"?, "read"?}`; `read` defaults to `/usr`, `/lib`, `/lib64`, `/bin`, `/etc`. |
| `start` / `Running` | Start one bot under a `ProcessExecutor` with the network open and a thread waiting on it; `stop()` kills the group, `outcome()` says how it ended, `wait()` blocks for that. |
| `Fleet` | A set of bots, all up or all down: the first that fails to start stops the rest and is reported by name. |

## Running

```sh
cargo build -p rusty_bot
target/debug/rusty-bot run bots.json /var/lib/rusty-bot
```

```json
[
  {"name": "tick", "program": "/opt/rusty_tick/rusty_tick", "workspace": "/srv/bots/tick",
   "args": ["--bind", "127.0.0.1:4001"], "env": {"TICK_TOKEN": "…"}},
  {"name": "echo", "program": "/usr/bin/python3", "workspace": "/srv/bots/echo",
   "args": ["agent.py"]}
]
```

`rusty-bot` is both the fleet runner and the sandbox helper: the executor
starts `rusty-bot __sandbox …` for each bot, a fresh single-threaded
process that confines itself and `exec`s the program. The state directory
holds the executor's status files and must be outside every workspace.
A bot's port is whatever its own arguments or environment say; point a
channel runner's `AGENT_URL`, or the gateway's `agui` route, at it.

## Dependencies

First-party only: `rusty_sandbox` and `rusty_json`.
