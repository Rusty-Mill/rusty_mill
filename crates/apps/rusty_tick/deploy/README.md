# Running rusty_tick as a service

One process serves the task API, MCP at `/mcp` and the web UI. It is supervised by systemd the
way Mill Board's and `rusty_remind_me`'s units are: a system unit, `Restart=always`, loopback
bind, the token in an owner-only file. Mill Board on the same host reads its cards from it
([ADR-0012](../../../../docs/adr/0012-one-task-store.md)).

| | |
|---|---|
| Unit | [`rusty-tick.service`](rusty-tick.service) |
| Settings | [`tick.env.example`](tick.env.example), copied to `~/.rusty_tick/tick.env` (mode 600) |
| Data | `~/.rusty_tick/data` |
| Web UI | `~/.rusty_tick/web` (a copy of `web/dist`) |
| Port | 8788, loopback; Mill Board keeps 8787 |
| Health | `GET /health` (no token) |

## Install

```sh
cargo build --release -p rusty_tick
sudo install -m 755 target/release/rusty_tick /usr/local/bin/
# The web UI links two local packages; build them first, as CI does.
for d in agui-core agui-react; do (cd crates/libs/protocol/rusty_agui/packages/$d && npm ci && npm run build); done
(cd crates/apps/rusty_tick/web && npm ci && npm run build)

mkdir -p ~/.rusty_tick/data && chmod 700 ~/.rusty_tick
cp -r crates/apps/rusty_tick/web/dist ~/.rusty_tick/web
install -m 600 crates/apps/rusty_tick/deploy/tick.env.example ~/.rusty_tick/tick.env
$EDITOR ~/.rusty_tick/tick.env        # user, paths, token

sudo install -m 644 crates/apps/rusty_tick/deploy/rusty-tick.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now rusty-tick
```

```sh
curl -fsS http://127.0.0.1:8788/health
journalctl -u rusty-tick -f
```

## With Mill Board on the same host

Install Mill Board from [its deploy guide](../../rusty_mill_board/deploy/README.md) with
`MILL_BOARD_TICK_URL=http://127.0.0.1:8788` and the same `RUSTY_TICK_TOKEN` in `board.env`.
`mill-board.service` starts after `rusty-tick.service` when both are installed. While rusty_tick
is down the board's `/health` answers 503 and writes fail; nothing is lost.

## Moving an existing data directory

Stop the old server first, so the copy is one moment of the store, then copy the whole directory
into `~/.rusty_tick/data` and keep the old one until the new server has run. Use the old token in
`tick.env`, or every client needs a new one. Point each client (Mill Board, `claude mcp add` entries,
the phone) at the new address.

## Reaching it from another machine

It speaks plain HTTP on loopback. From your own machine use an SSH tunnel
(`ssh -L 8788:127.0.0.1:8788 <host>`, then `http://127.0.0.1:8788`) or TLS in front. Agents use Mill
Board, not rusty_tick, so rusty_tick never needs a public address.

## Backups, upgrade and rollback

Back up `~/.rusty_tick/data` with the service stopped, and copy it off the host. Replace
`/usr/local/bin/rusty_tick` and `systemctl restart rusty-tick` to upgrade; keep the previous binary
beside it (`rusty_tick.prev`) to roll back.
