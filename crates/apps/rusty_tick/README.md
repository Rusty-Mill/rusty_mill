# rusty_tick

A self-hosted, TickTick-style task manager built on
[`rusty_multimodal_db_engine`](../../libs/storage/rusty_multimodal_db_engine).
A clean-room design: it is written from the product's public behaviour and
documented API, not from its code.

Status: storage, a JSON HTTP API for one user or several, and a web UI in [`web/`](web/README.md)
(lists, tags, Kanban, timeline and Eisenhower-matrix views, saved filters, Won't Do, calendar (month, week, day, 3-day, ten-day, agenda) with `.ics` import and feed subscriptions,
focus timer with ambient sound, pomo estimates and interruption tracking, countdowns, assignees, habits,
summaries). Sync is by polling; there is no push channel.
Engine findings are in [SPIKE-FINDINGS.md](SPIKE-FINDINGS.md) (issue
[#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)); the HTTP stack
choice is in [ADR-0001](docs/decisions/ADR-0001-http-stack.md); the
proposed per-user token design is in
[ADR-0002](docs/decisions/ADR-0002-per-user-tokens.md).

## Run

```
RUSTY_TICK_TOKEN=<16+ characters> cargo run -p rusty_tick -- \
    [--data-dir rusty_tick_data] [--addr 127.0.0.1:8787] [--web-dir web/dist] [--allow-remote]

curl -H "Authorization: Bearer $RUSTY_TICK_TOKEN" -d '{"name":"Inbox"}' localhost:8787/api/v1/lists
```

The server speaks plain HTTP and refuses a non-loopback `--addr` unless
`--allow-remote` is given (put TLS in front of it).

### Several users

If `<data-dir>/users.json` exists, the server runs for several users
([ADR-0002](docs/decisions/ADR-0002-per-user-tokens.md)): each token is
`<user key>.<secret>`, each user's data lives in `<data-dir>/users/<key>/`, at
most 32 users are open at once, and `RUSTY_TICK_TOKEN` must not be set. The
file is re-read as it changes, so a revoked token stops working without a
restart. Every refusal is the same bare `401`.

```
rusty_tick user add alice phone --data-dir DIR   # prints alice's token, once
rusty_tick user list | revoke KEY TOKEN_ID | disable KEY | enable KEY
rusty_tick user adopt alice --data-dir DIR       # move a single-user store into alice
```

`add` creates `users.json` in a fresh directory (and adds a token to an
existing user). It refuses a directory that already holds a single-user
store, which a `users.json` would hide; `adopt` moves that store into a user
(server stopped) and prints their token. Commands take `users.lock`, so two at
once cannot lose a write. A running server picks the change up within a second.

Either way the server locks the data directory it opens, so a second
`rusty_tick` on the same directory refuses to start.

## API

JSON, camelCase, `Authorization: Bearer <token>` on everything but `/health`.
Errors are `{"error":{"code","message"}}`: 400 malformed request, 401, 404,
422 invalid value, 500. Inputs reject unknown fields.

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | no auth |
| GET | `/api/v1/snapshot` | lists, tags and tasks in one read |
| GET, POST | `/api/v1/lists` | `{"name"}` |
| GET, PATCH, DELETE | `/api/v1/lists/{id}` | PATCH `{"name","color","archived","viewMode","sortType","sortOrder"}` (`viewMode`: `list`, `kanban`, `timeline`, `matrix`); delete removes its tasks |
| GET | `/api/v1/lists/{id}/tasks` | `?status=open\|done\|wontdo&sort=manual\|due` |
| POST | `/api/v1/tasks` | `{"listId","title","notes","priority","startMs","dueMs","isAllDay","timeZone","reminders","repeatFlag","items","tags","parentId","status"}`; `status` (default `open`) creates it closed in one write |
| GET, PATCH, DELETE | `/api/v1/tasks/{id}` | PATCH: absent = keep, `"dueMs":null` = clear; delete removes subtasks |
| POST | `/api/v1/tasks/{id}/complete`, `/reopen`, `/restore` | `DELETE /tasks/{id}` moves to the trash; `?purge=true` removes it for good |
| DELETE | `/api/v1/trash` | empties the trash |
| PUT | `/api/v1/tasks/{id}/order` | `{"sortOrder": n}`: one durable slot write |
| GET | `/api/v1/search?q=` | any term, as a prefix, in title or notes (`grocer` finds `groceries`) |
| GET, POST | `/api/v1/tags` | PATCH `/tags/{name}`, POST `/tags/{name}/rename` |
| GET | `/api/v1/tags/{tag}/tasks` | |
| POST | `/api/v1/fetch-ics` | `{"url"}` → `{"text"}`: the server fetches a calendar feed for the web UI (browsers cannot read most). `https://` or `webcal://`, port 443, public addresses only (checked before connecting, redirects re-checked), 4 MiB, 6 s per step; 422 for a URL it refuses, 502 for a feed that fails, 503 when 4 fetches are already running. It runs after the server's request lock is released, so a slow feed holds up no other request |
| GET | `/api/v1/docs/{kind}` | client documents: `GET`/`PUT`/`DELETE /docs/{kind}/{id}`; kinds `habit`, `habit_checkin`, `focus`, `prefs`, `summary_template`, `comment`, `filter`, `countdown`, `estimate`, `assignee`, `subscription`, `import_alias`, `board`, `board_number`. A document's `etag` is its `updatedMs`; `PUT` takes `If-Match` and `If-None-Match: *`. A `board` document (Mill Board, [ADR-0012](../../../docs/adr/0012-one-task-store.md)) has its task's id, needs that task out of the trash, is written with `If-Task-Match: <task etag>` as well, and is purged with the task. A `board_number` document is Mill Board's reservation of one card number, created once with `If-None-Match: *` and kept |
| GET | `/api/v1/moves?since=<ms>&after=<seq>&limit=<n>` | Mill Board column history, oldest first: `{"moves": [{"seq", "taskId", "from", "to", "atMs", "actor"}], "next"}`. A task's column is read as Mill Board reads it: `null` without the `agent` tag, `done` once closed, else its first `board:<col>` tag if that names a board column, else `backlog`. Every write that changes it appends a move, in the same request, as the `X-Actor` header names (`tick` without one; tag rename and delete take it too). `since` is inclusive, `after` is a `seq` (exclusive), `limit` is at most 1000; pass `next` as `after` until it is `null`. `GET /api/v1/tasks/{id}/moves` gives one task's. History starts at deploy and is never deleted, a purged task's included, so a `seq` is never reused. Each write's moves are first made durable in `moves.intent`; a crash or failed history write refuses task writes until restart, which replays it |
| GET | `/api/v1/smart/today`, `/next7`, `/overdue` | `?utcOffsetMin=` (default 0); open tasks in non-archived lists |

Priority is 0, 1, 3 or 5. A task's status is `open`, `done` or `wontdo` (PATCH `status`; closing stamps `completedMs`, reopening clears it). Times are Unix milliseconds. Subtasks nest one level
and stay in their parent's list (moving a parent moves them). `repeatFlag` is an RFC 5545
`RRULE` and `reminders` are `TRIGGER:` strings; the server stores both, and the web UI expands
recurrence and raises reminders as browser notifications.

## MCP

The server also speaks MCP (Streamable HTTP) at `/mcp`, under the same bearer
tokens as the API, so an agent can read and change your tasks. Tools:
`list_lists`, `list_tasks`, `due_tasks`, `search_tasks`, `get_task`,
`add_task` (to the Inbox unless `listId` is given), `update_task`,
`complete_task`, `reopen_task` and `delete_task` (to the Trash). Each runs
the matching API route in-process, with its validation and errors. Browsers
are refused (no `Origin` is accepted). For Claude Code:

```
claude mcp add --transport http tick http://127.0.0.1:8787/mcp --header "Authorization: Bearer $RUSTY_TICK_TOKEN"
```

With `RUSTY_TICK_ISSUER` set to the server's public https origin, a client can
sign in with OAuth instead (a claude.ai connector's "Sign in now"); see
[deploy/README.md](deploy/README.md#signing-in-from-claudeai-oauth).

The web UI's assistant (`/api/agent`) offers the same tools, by the same
names, run in the browser. It is rule-based, not a model: “add buy milk”,
“find milk”, “what's due today?”, “what's overdue?”, “due this week”,
“complete buy milk”, “reopen buy milk”, “delete buy milk”, “rename buy milk to
buy oat milk”, “move buy milk to friday” (also tomorrow, in 3 days, next week,
2026-11-05, none, with a time: “friday at 3pm”, “tomorrow 9:30”; or a time alone:
“3pm”, “at 9”, “all day”), “make buy milk high priority” (or medium, low, no), “move buy
milk to list Work” (or “to the Work list”, “to inbox”), “repeat water plants
every 2 weeks” (or daily, weekly, monthly, yearly, on weekdays, every monday and
friday; “make … repeat …”; “stop repeating …”), “remind me about dentist 1 hour
before” (or 5 minutes, a day before, at the time, on the day; “stop reminding me about
…”; an all-day task is reminded at 09:00), “snooze dentist for 2 hours” (or 30 minutes,
a day, 1 week, “until friday at 9”: minutes and hours count from now, days from the due
day), “tag dentist with health” (several: “#work, home and errands”; “remove tag health from
dentist”), “what's high priority today?” (or this week, overdue, or any time; low, medium,
no priority), “tasks tagged work” (or “#work”), “what's in Work?”, “done tasks” (or completed, won't do, open; combined: “done high priority
tasks tagged work in Home”),
“lists”, and for habits “habits”, “check in meditate”, “undo check in meditate”
and “add habit meditate” (habits live in the browser, so these have no MCP tool). A task is named by its title; when several
match, it asks which. Quote a name that contains “to” (“rename "a to b" to c”). An
unreadable day changes nothing.

## Layout

`api` (pure router) and `dto` (wire shapes) sit over `service` (rules), which
sits over `store`/`lists` (the engine). `server` binds `Backend` to
`rusty_serve`, the shared blocking HTTP server that also serves `web/dist`;
`mcp` routes `/mcp` beside it and turns tool calls into API requests; `oauth`
adapts `rusty_oauth_server`'s sign-in endpoints to it.

```
cargo test -p rusty_tick
cargo test -p rusty_tick --release --test spike -- --ignored --nocapture   # scale probe
```
