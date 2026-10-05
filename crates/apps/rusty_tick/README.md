# rusty_tick

A self-hosted, TickTick-style task manager built on
[`rusty_multimodal_db_engine`](../../libs/storage/rusty_multimodal_db_engine).
A clean-room design: it is written from the product's public behaviour and
documented API, not from its code.

Status: storage, a JSON HTTP API for one user or several, and a web UI in [`web/`](web/README.md)
(lists, tags, Kanban, timeline and Eisenhower-matrix views, saved filters, Won't Do, calendar with
`.ics` import, focus timer with pomo estimates and interruption tracking, countdowns, habits, summaries). Sync is by polling; there is no push channel.
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
| POST | `/api/v1/tasks` | `{"listId","title","notes","priority","startMs","dueMs","isAllDay","timeZone","reminders","repeatFlag","items","tags","parentId"}` |
| GET, PATCH, DELETE | `/api/v1/tasks/{id}` | PATCH: absent = keep, `"dueMs":null` = clear; delete removes subtasks |
| POST | `/api/v1/tasks/{id}/complete`, `/reopen`, `/restore` | `DELETE /tasks/{id}` moves to the trash; `?purge=true` removes it for good |
| DELETE | `/api/v1/trash` | empties the trash |
| PUT | `/api/v1/tasks/{id}/order` | `{"sortOrder": n}`: one durable slot write |
| GET | `/api/v1/search?q=` | any term, as a prefix, in title or notes (`grocer` finds `groceries`) |
| GET, POST | `/api/v1/tags` | PATCH `/tags/{name}`, POST `/tags/{name}/rename` |
| GET | `/api/v1/tags/{tag}/tasks` | |
| GET | `/api/v1/docs/{kind}` | client documents: `PUT`/`DELETE /docs/{kind}/{id}`; kinds `habit`, `habit_checkin`, `focus`, `prefs`, `summary_template`, `comment`, `filter`, `countdown`, `estimate` |
| GET | `/api/v1/smart/today`, `/next7`, `/overdue` | `?utcOffsetMin=` (default 0); open tasks in non-archived lists |

Priority is 0, 1, 3 or 5. A task's status is `open`, `done` or `wontdo` (PATCH `status`; closing stamps `completedMs`, reopening clears it). Times are Unix milliseconds. Subtasks nest one level
and stay in their parent's list (moving a parent moves them). `repeatFlag` is an RFC 5545
`RRULE` and `reminders` are `TRIGGER:` strings; the server stores both, and the web UI expands
recurrence and raises reminders as browser notifications.

## Layout

`api` (pure router) and `dto` (wire shapes) sit over `service` (rules), which
sits over `store`/`lists` (the engine). `server` binds `Backend` to
`rusty_serve`, the shared blocking HTTP server that also serves `web/dist`.

```
cargo test -p rusty_tick
cargo test -p rusty_tick --release --test spike -- --ignored --nocapture   # scale probe
```
