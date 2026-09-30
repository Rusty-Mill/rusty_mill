# rusty_tick

A self-hosted, TickTick-style task manager built on
[`rusty_multimodal_db_engine`](../../libs/storage/rusty_multimodal_db_engine).
A clean-room design: it is written from the product's public behaviour and
documented API, not from its code.

Status: storage plus a JSON HTTP API, for one user or several. No web UI or sync yet.
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
| GET, POST | `/api/v1/lists` | `{"name"}` |
| GET, PATCH, DELETE | `/api/v1/lists/{id}` | PATCH `{"name","archived"}`; delete removes its tasks |
| GET | `/api/v1/lists/{id}/tasks` | `?status=open\|done&sort=manual\|due` |
| POST | `/api/v1/tasks` | `{"listId","title","notes","priority","dueMs","tags","parentId"}` |
| GET, PATCH, DELETE | `/api/v1/tasks/{id}` | PATCH: absent = keep, `"dueMs":null` = clear; delete removes subtasks |
| POST | `/api/v1/tasks/{id}/complete`, `/reopen` | |
| PUT | `/api/v1/tasks/{id}/order` | `{"sortOrder": n}`: one durable slot write |
| GET | `/api/v1/search?q=` | any term, as a prefix, in title or notes (`grocer` finds `groceries`) |
| GET | `/api/v1/tags/{tag}/tasks` | |
| GET | `/api/v1/smart/today`, `/next7`, `/overdue` | `?utcOffsetMin=` (default 0); open tasks in non-archived lists |

Priority is 0, 1, 3 or 5. Times are Unix milliseconds. Subtasks nest one level
and stay in their parent's list (moving a parent moves them). Not yet:
recurrence, reminders.

## Layout

`api` (pure router) and `dto` (wire shapes) sit over `service` (rules), which
sits over `store`/`lists` (the engine). `server` is the only file that touches
a socket.

```
cargo test -p rusty_tick
cargo test -p rusty_tick --release --test spike -- --ignored --nocapture   # scale probe
```
