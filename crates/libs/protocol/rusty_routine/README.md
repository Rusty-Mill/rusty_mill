# rusty_routine

Routines: an [AG-UI](https://docs.ag-ui.com) agent run on a schedule. A
routine is a cron expression, a prompt and a failure budget. When it is
due, it builds the `RunAgentInput` to post: a fresh thread per firing with
the prompt as the person's message, and the routine's name and scheduled
time in `forwardedProps` so a gateway rule can tell a routine's run from a
person's. Runs that fail in a row count down the budget; at zero the
routine is disabled rather than failing on every tick until someone
notices. A success resets the count.

Step 6 of the [ADR-0007 follow-ons](../../../../docs/dev_phases/ADR_0007/FOLLOW-ONS.md).

## Shape

| Piece | What it does |
| --- | --- |
| `Schedule` | Five-field cron in UTC (`minute hour day month weekday`): `*`, numbers, ranges, lists, steps; `0` and `7` are Sunday; when both day fields are set, either matches. `next_after(t)` is the first scheduled minute strictly after `t`, or `None` when nothing comes within five years (`30 2`). A wrong expression fails at parse time. |
| `Routine` | `new(name, schedule, prompt, max_failures, now)`; `due(now)`; `fire(now)` returns the run and advances the schedule (a missed firing is skipped, not made up); `record(succeeded)` counts the outcome and disables at the budget. |
| `load(json, now)` | Routines from a JSON array of `{"name", "cron", "prompt", "maxFailures"?}` (default budget 3); names must be unique. |
| `run::tick` / `run::run` (feature `run`) | Fire every due routine against one `HttpAgent`, read the reply back through `rusty_channel::Thread`, report one line per run; the loop sleeps until the earliest next firing and stops when every routine is disabled. |

Sans-IO, like `rusty_channel`: nothing in the library reads a clock,
sleeps or opens a socket, so every rule is tested against fixed times.

## Running

```sh
ROUTINES=routines.json AGENT_URL=http://127.0.0.1:8080/api/agent \
  cargo run -p rusty_routine --features run --example routines
```

```json
[
  {"name": "digest", "cron": "0 9 * * 1-5", "prompt": "What's due today?"},
  {"name": "ping", "cron": "*/5 * * * *", "prompt": "ping", "maxFailures": 1}
]
```

Optional: `AGENT_TOKEN`, sent as a bearer token, so `rusty_agent_gateway`'s
`agui` route sees the routine as its requester; a rule such as
`agui.forwardedProps.routine == "digest"` can then permit it and nothing
else. State (next firing, failures, disabled) lives in memory for the life
of the process.

## Dependencies

First-party only: `rusty_agui` (the run input), `rusty_channel` (reading
the reply back), `rusty_json` (the routines file). The `run` feature adds
`rusty_agui`'s client.
