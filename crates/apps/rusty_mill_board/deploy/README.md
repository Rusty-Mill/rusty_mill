# Running Mill Board as a service

One process serves the UI, the web API and MCP over HTTP. The cards live in rusty_tick
(`MILL_BOARD_TICK_URL`, ADR-0012), so run rusty_tick first. It is supervised by systemd the way `rusty_remind_me`'s units are: a system unit, `Restart=always`,
loopback bind, bearer tokens in owner-only files.

| | |
|---|---|
| Unit | [`mill-board.service`](mill-board.service) |
| Settings | [`board.env.example`](board.env.example), copied to `~/.mill-board/board.env` (mode 600) |
| Data | the cards in rusty_tick; `~/.mill-board/settings/` here |
| Port | 8787, loopback by default |
| Health | `GET /health`: 200 while rusty_tick answers, 503 while it does not |
| Stop | SIGTERM (`systemctl stop`) stops accepting, waits up to 10 s for running requests, then exits 0 |

## Install

```sh
cargo build --release -p rusty_mill_board
sudo install -m 755 target/release/rusty_mill_board /usr/local/bin/

mkdir -p ~/.mill-board && chmod 700 ~/.mill-board
install -m 600 crates/apps/rusty_mill_board/deploy/board.env.example ~/.mill-board/board.env
$EDITOR ~/.mill-board/board.env      # user, rusty_tick URL and token, bind address, allowed hosts

sudo install -m 644 crates/apps/rusty_mill_board/deploy/mill-board.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now mill-board
```

The first start creates `settings/identities.json` with an `admin` agent. Because the service's
stderr is the journal, its token is written to `settings/first-admin.token` (mode 600) and only
the path is logged. Read it, open `http://127.0.0.1:8787/#token=<it>`, add one agent per model in
the settings view, then delete the file.

```sh
curl -fsS http://127.0.0.1:8787/health        # {"ok":true}
journalctl -u mill-board -f
```

Register the board in each model's client as in [`../docs/connecting-models.md`](../docs/connecting-models.md).

## Restarts and backups

The board keeps no card state of its own: cards, claims, comments and dependencies are in rusty_tick,
so a restart or crash loses none of them. Events (`board_events`) live in the process and start over
after a restart. `StartLimitIntervalSec=0` keeps systemd retrying forever.

Back up rusty_tick's data directory, and `settings/` here (owner-only: it holds tokens). Copy both
off the host so a lost disk does not take them all.

## Upgrade and rollback

Replace `/usr/local/bin/rusty_mill_board` and `systemctl restart mill-board`. Keep the previous
binary beside it (`rusty_mill_board.prev`) to roll back.

## Not covered

- TLS: the server speaks plain HTTP. Over a LAN put a TLS proxy in front, or stay on loopback.
- `mill-runner` (the optional stage runner) is a separate process and not part of this unit.
