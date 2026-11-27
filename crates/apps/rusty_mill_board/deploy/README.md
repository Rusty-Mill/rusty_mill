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

## Signing in from claude.ai (OAuth)

A claude.ai custom connector signs in with OAuth instead of a pasted bearer token. Publish the
board at an https origin through TLS in front (it can still bind loopback), set
`MILL_BOARD_ISSUER` in `board.env` to that origin, e.g. `https://board.baileyfam.us`, and restart.
Its host is accepted as a `Host` name without adding it to `MILL_BOARD_ALLOWED_HOSTS`. Add
`https://board.baileyfam.us/mcp` as the connector's URL; "Sign in now" opens a consent page that
asks for one of the board's own tokens. The grant then acts as that token's agent, with the
agent's role and projects as they are at each request.

- Existing bearer tokens keep working, OAuth on or off.
- Grants are kept in `settings/oauth.json` (owner-only), which is bound to the issuer on first
  start: a board started with another issuer on the same file refuses to start, and so does one
  pointed at an existing file that names no issuer (even an empty one) — give it a new file
  instead. A running board checks the binding on every access,
  so a file replaced or claimed by another board meanwhile serves no grants and takes no writes. Removing an agent in settings suspends
  their grants; adding an agent of the same name again restores them. Rotating an agent's token
  does not affect their grants. To end every grant, stop the board, delete `oauth.json` and start
  it again; a connector's own disconnect revokes just its grant.
- An open board (no identities, no `--auth-token`) refuses to start with an issuer: there is no
  token to approve with.

## Restarts and backups

The board keeps no card state of its own: cards, claims, comments and dependencies are in rusty_tick,
so a restart or crash loses none of them. Events (`board_events`) live in the process and start over
after a restart. `StartLimitIntervalSec=0` keeps systemd retrying forever.

Back up rusty_tick's data directory, and `settings/` here (owner-only: it holds tokens and, with
OAuth, the connectors' grants). Copy both
off the host so a lost disk does not take them all.

### SQLite store (`--db`)

With `MILL_BOARD_DB` the cards are one WAL file. `backup.sh` takes a consistent copy with SQLite's
online backup (the board keeps running; never `cp` the live file), requires `PRAGMA integrity_check`
= `ok` on the copy, gzips it, keeps the newest `MILL_BOARD_BACKUP_KEEP`, and mirrors them with
rsync to `MILL_BOARD_BACKUP_DEST/<host>/`. A copy counts only as a verified pair (gzip plus its `.sha256`,
published last, and the gzip must match it): retention, shipping and pruning ignore a lone gzip, and the next snapshot removes
it. The destination is pruned only after a checksum comparison shows its copies equal the verified local pairs, and never when this host
holds fewer copies than it should and fewer than the destination (a wiped local directory). A
failed shipment prunes nothing, and runs are serialized by a lock in the backup directory. Set `MILL_BOARD_BACKUP_REQUIRE_MOUNT` to refuse to ship when the
destination's mount is absent. The unit is sandboxed (`ProtectSystem=strict`): it may write
`~/.mill-board` and `/mnt/beast/mill-board`; for any other destination add a `ReadWritePaths=`
drop-in. The destination directory must already exist (rsync makes only the `<host>/` level).

```sh
sudo install -m 755 crates/apps/rusty_mill_board/deploy/backup.sh /usr/local/bin/mill-board-backup
install -m 600 crates/apps/rusty_mill_board/deploy/backup.env.example ~/.mill-board/backup.env
$EDITOR ~/.mill-board/backup.env
sudo install -m 644 crates/apps/rusty_mill_board/deploy/mill-board-backup.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now mill-board-backup.timer
sudo systemctl start mill-board-backup.service && journalctl -u mill-board-backup -n 20   # try once
```

`settings/` (tokens, OAuth grants) is not in the SQLite file: copy it off the host too.

**Restore** (try it on a copy first; the live file and its `-wal`/`-shm` are kept together, never deleted):

```sh
f=mill-board-<stamp>.sqlite3.gz
sha256sum -c $f.sha256 && gunzip -k $f
sqlite3 mill-board-<stamp>.sqlite3 'pragma integrity_check'      # ok (or python3 -c, no sqlite3 CLI needed)
# trial: serve the copy on a spare port, count the cards in the UI
rusty_mill_board --db mill-board-<stamp>.sqlite3 --http 127.0.0.1:8799
# real restore: stop the board, move the old file and its sidecars aside as one set, put the copy in place
sudo systemctl stop mill-board
d=~/.mill-board; keep=$d/restore-aside-$(date -u +%Y%m%dT%H%M%SZ); mkdir "$keep"
mv "$d"/mill-board.sqlite3* "$keep"/        # the .sqlite3, -wal and -shm together
cp mill-board-<stamp>.sqlite3 $d/mill-board.sqlite3
sudo systemctl start mill-board             # roll back: stop, move the copy out, move $keep/* back
```

## Upgrade and rollback

Upgrading from a board file (`MILL_BOARD_STORE`): move the cards into rusty_tick first
(`tools/migrate_artifact_board.py` for the artifact board), then replace `MILL_BOARD_STORE` and any
`MILL_BOARD_BACKUP_*` lines in `board.env` with `MILL_BOARD_TICK_URL` and `RUSTY_TICK_TOKEN`. The
service refuses to start while a retired line is present, rather than coming up empty in memory.

Replace `/usr/local/bin/rusty_mill_board` and `systemctl restart mill-board`. Keep the previous
binary beside it (`rusty_mill_board.prev`) to roll back.

## Not covered

- TLS: the server speaks plain HTTP. Over a LAN put a TLS proxy in front, or stay on loopback.
- `mill-runner` (the optional stage runner) is a separate process and not part of this unit.
