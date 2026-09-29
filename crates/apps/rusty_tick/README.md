# rusty_tick

A self-hosted, TickTick-style task manager built on
[`rusty_multimodal_db_engine`](../../libs/storage/rusty_multimodal_db_engine).
A clean-room design: it is written from the product's public behaviour and
documented API, not from its code.

Status: storage plus a JSON HTTP API. No web UI, sync or multi-user yet.
Engine findings are in [SPIKE-FINDINGS.md](SPIKE-FINDINGS.md) (issue
[#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)); the HTTP stack
choice is in [ADR-0001](docs/decisions/ADR-0001-http-stack.md); the
proposed per-user token design is in
[ADR-0002](docs/decisions/ADR-0002-per-user-tokens.md).

## Run

```
RUSTY_TICK_TOKEN=<16+ characters> cargo run -p rusty_tick -- \
    [--data-dir rusty_tick_data] [--addr 127.0.0.1:8787] [--allow-remote]

curl -H "Authorization: Bearer $RUSTY_TICK_TOKEN" -d '{"name":"Inbox"}' localhost:8787/api/v1/lists
```

The server speaks plain HTTP and refuses a non-loopback `--addr` unless
`--allow-remote` is given (put TLS in front of it).

## API

JSON, camelCase, `Authorization: Bearer <token>` on everything but `/health`.
Errors are `{"error":{"code","message"}}`: 400 malformed request, 401, 404,
422 invalid value, 500. Inputs reject unknown fields.

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | no auth |
| GET, POST | `/api/v1/lists` | `{"name"}` |
| GET, PATCH, DELETE | `/api/v1/lists/{id}` | PATCH `{"name","archived"}`; delete removes its tasks |
| GET | `/api/v1/lists/{id}/tasks` | `?status=open\|done&sort=manual\|due` |
| POST | `/api/v1/tasks` | `{"listId","title","notes","priority","dueMs","tags","parentId"}` |
| GET, PATCH, DELETE | `/api/v1/tasks/{id}` | PATCH: absent = keep, `"dueMs":null` = clear; delete removes subtasks |
| POST | `/api/v1/tasks/{id}/complete`, `/reopen` | |
| PUT | `/api/v1/tasks/{id}/order` | `{"sortOrder": n}`: one durable slot write |
| GET | `/api/v1/search?q=` | whole-word match on title and notes (no prefix search yet) |
| GET | `/api/v1/tags/{tag}/tasks` | |
| GET | `/api/v1/smart/today`, `/next7`, `/overdue` | `?utcOffsetMin=` (default 0); open tasks in non-archived lists |

Priority is 0, 1, 3 or 5. Times are Unix milliseconds. Subtasks nest one level
and stay in their parent's list. Not yet: moving a task between lists,
recurrence, reminders, accounts.

## Layout

`api` (pure router) and `dto` (wire shapes) sit over `service` (rules), which
sits over `store`/`lists` (the engine). `server` is the only file that touches
a socket.

```
cargo test -p rusty_tick
cargo test -p rusty_tick --release --test spike -- --ignored --nocapture   # scale probe
```
