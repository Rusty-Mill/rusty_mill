# Running Mill Board as a service

One process owns the board file and serves the UI, the web API and MCP over HTTP. It is
supervised by systemd the way `rusty_remind_me`'s units are: a system unit, `Restart=always`,
loopback bind, bearer tokens in owner-only files.

| | |
|---|---|
| Unit | [`mill-board.service`](mill-board.service) |
| Settings | [`board.env.example`](board.env.example), copied to `~/.mill-board/board.env` (mode 600) |
| Data | `~/.mill-board/board.mmap` (+ `.records`, `.inserts`, `.lock`), `settings/`, `backups/` |
| Port | 8787, loopback by default |
| Health | `GET /health`: 200 while the store can commit, 503 once it needs a restart |
| Stop | SIGTERM (`systemctl stop`) ends the accept loop; the process exits cleanly |

## Install

```sh
cargo build --release -p rusty_mill_board
sudo install -m 755 target/release/rusty_mill_board /usr/local/bin/

mkdir -p ~/.mill-board && chmod 700 ~/.mill-board
install -m 600 crates/apps/rusty_mill_board/deploy/board.env.example ~/.mill-board/board.env
$EDITOR ~/.mill-board/board.env      # user, paths, bind address, allowed hosts

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

## Restarts

Every commit is one durable record, and the board's state is rebuilt from the records at open, so
cards, claims and the event cursor survive any restart or crash. `StartLimitIntervalSec=0` keeps
systemd retrying forever. A second process on the same `--store` is refused ("in use by another
process"), so a stray stdio instance cannot diverge from the service.

A failed write whose outcome is unknown puts the store in a refuse-writes state until it is
reopened. `/health` then answers 503. Run `sudo systemctl restart mill-board`: the reopen settles
what really committed.

## Backups and restore

With `MILL_BOARD_BACKUP_DIR` set the service copies the store and the settings files into
`backups/board-<unix ms>/` every `MILL_BOARD_BACKUP_SECS` and keeps the newest
`MILL_BOARD_BACKUP_KEEP`. No commit lands while the store files are copied, so a backup opens as the
board stood at one moment, and a backup is renamed into place only when complete. The store is
compacted after each backup. The settings files are copied owner-only: they hold the tokens.
Copy `backups/` off the host so a lost disk does not take both.

Restore:

```sh
sudo systemctl stop mill-board
cd ~/.mill-board && mkdir -p replaced && mv board.mmap* replaced/
cp -p backups/board-<time>/board.mmap* .
# settings: only if they were lost; the backup holds identities.json, projects.json, ...
sudo systemctl start mill-board
```

## Upgrade and rollback

Replace `/usr/local/bin/rusty_mill_board` and `systemctl restart mill-board`. Keep the previous
binary beside it (`rusty_mill_board.prev`) to roll back; the on-disk format is versioned by the
row schema tag, so a rollback across a format change needs the backup taken before it.

## Not covered

- TLS: the server speaks plain HTTP. Over a LAN put a TLS proxy in front, or stay on loopback.
- `mill-runner` (the optional stage runner) is a separate process and not part of this unit.
