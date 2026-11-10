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

mkdir -p ~/.rusty_tick && chmod 700 ~/.rusty_tick
cp -r crates/apps/rusty_tick/web/dist ~/.rusty_tick/web     # first install only; see Upgrade
install -m 600 crates/apps/rusty_tick/deploy/tick.env.example ~/.rusty_tick/tick.env
$EDITOR ~/.rusty_tick/tick.env        # user, paths, token

sudo install -m 644 crates/apps/rusty_tick/deploy/rusty-tick.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable rusty-tick
# Moving an existing store? Put it in place now (below), before the first start.
mkdir -p ~/.rusty_tick/data           # a fresh store otherwise
sudo systemctl start rusty-tick
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

The new store must be a complete snapshot, never copied into a store a server has already opened.

1. Stop the old server, so its directory is one moment of the store. Keep it as it is: it is the
   rollback until the new one checks out.
2. Copy the whole directory to the host (say `~/rusty_tick_data.snapshot`).
3. Set the tokens in `~/.rusty_tick/tick.env` before the server opens the store, because it reads
   them only at start. A single-user store: put the old server's token in `RUSTY_TICK_TOKEN`, so
   clients keep working. A store with `users.json`: delete the `RUSTY_TICK_TOKEN` line; each user's
   tokens come with the directory.
4. If `rusty-tick` has started here, stop it and move its directory aside, don't merge into it.
   Then put the snapshot in place and start:

   ```sh
   sudo systemctl stop rusty-tick
   [ -e ~/.rusty_tick/data ] && mv ~/.rusty_tick/data ~/.rusty_tick/data.replaced-$(date +%s)
   mv ~/rusty_tick_data.snapshot ~/.rusty_tick/data
   sudo systemctl start rusty-tick
   ```

5. Check the new server before any client moves: `/health`, then the task counts through the API
   (`GET /api/v1/snapshot`) against the old server's.
6. Then point each client (Mill Board, `claude mcp add` entries, the phone) at the new address.

## Reaching it from another machine

It speaks plain HTTP on loopback. From your own machine use an SSH tunnel
(`ssh -L 8788:127.0.0.1:8788 <host>`, then `http://127.0.0.1:8788`) or TLS in front.

## Signing in from claude.ai (OAuth)

A claude.ai custom connector signs in with OAuth instead of a pasted bearer token. Publish the
server at an https origin through TLS in front (it still binds loopback), then set
`RUSTY_TICK_ISSUER` in `tick.env` to that origin, e.g. `https://tick.baileyfam.us`, and restart.
Add `https://tick.baileyfam.us/mcp` as the connector's URL; "Sign in now" opens a consent page
that asks for one of the server's own tokens (the shared token, or a user's `<key>.<secret>`).
The grant then acts as that token's user.

- Existing bearer tokens keep working everywhere, OAuth on or off.
- Grants are kept in `<data-dir>/oauth.json` and checked against `users.json` on every request.
  Disabling a user suspends their grants (within a second) but does not delete them: enabling
  the user again restores them.
- Revoking the token that approved a grant, or changing `RUSTY_TICK_TOKEN`, does not affect the
  grant. To end every grant, stop the server, delete `oauth.json` and start it again; a
  connector's own disconnect revokes just its grant.
- Back up `oauth.json` with the rest of the data directory, or every connector signs in again.

## From one shared token to a token per client

A single-user store has one token, `RUSTY_TICK_TOKEN`, which every client shares. Switching the
directory to users (`users.json`) gives each client its own token, revoked on its own. Adopt the
store as the user `default`: that is the subject single-user OAuth grants already carry, so
connectors stay signed in, and `oauth.json` stays where it is.

```sh
sudo systemctl stop mill-board rusty-tick
cp -a ~/.rusty_tick/data ~/.rusty_tick/data.pre-users     # and copy it off the host
rusty_tick user adopt default --data-dir ~/.rusty_tick/data           # token for the first client
rusty_tick user add default mill-board --data-dir ~/.rusty_tick/data  # one per further client
sed -i '/^RUSTY_TICK_TOKEN=/d' ~/.rusty_tick/tick.env    # the server refuses to start with it set
$EDITOR ~/.mill-board/board.env       # RUSTY_TICK_TOKEN=<the mill-board token>
sudo systemctl start rusty-tick mill-board
rusty_tick user list --data-dir ~/.rusty_tick/data      # token ids, for `user revoke default <id>`
```

Each token is printed once. The old shared token stops working at once, so give every client that
used it a token of its own. Roll back by stopping both services, putting `data.pre-users` back and
restoring the two env files.

## Backups, upgrade and rollback

### Backups

`rusty-tick-backup.timer` runs nightly at 03:30. It stops rusty-tick for a few seconds and
archives the whole data directory (users, OAuth grants, move history). Then it starts tick
again, if it was running, and mirrors the archives off the host with rsync into
`$RUSTY_TICK_BACKUP_DEST/<hostname>/` (the destination itself must exist). Mill Board answers 503 while tick is stopped. The newest
`RUSTY_TICK_BACKUP_KEEP` archives are kept, each with a `.sha256`. If this host has fewer
archives than that and fewer than the destination (a wiped local directory), the copy adds
without pruning the destination. A run that takes over an hour fails.

A copy of a running store can catch a write halfway. So `backup.sh snapshot` refuses while
`RUSTY_TICK_ADDR` answers or any lock in the data directory is held. It then holds those locks
until the archive is written. To run it by hand, stop rusty-tick first.

Install, after setting `RUSTY_TICK_BACKUP_DIR`, `RUSTY_TICK_BACKUP_KEEP` and
`RUSTY_TICK_BACKUP_DEST` in `tick.env` (see `tick.env.example`). The destination is any rsync
target off this host: `user@host:/path` over SSH, with a key the tick account can use without a
prompt, or a mounted share.

```sh
sudo install -m 755 crates/apps/rusty_tick/deploy/backup.sh /usr/local/bin/rusty-tick-backup
sudo install -m 755 crates/apps/rusty_tick/deploy/restore-drill.sh /usr/local/bin/rusty-tick-restore-drill
sudo cp crates/apps/rusty_tick/deploy/rusty-tick-backup.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl start rusty-tick-backup.service   # a first run now; check it
journalctl -u rusty-tick-backup -n 20 --no-pager
sudo systemctl enable --now rusty-tick-backup.timer
```

A failed run shows in `systemctl --failed` and the journal. Whatever failed, the unit starts
rusty-tick again.

**Restore drill.** Run it after installing and then monthly. It never touches the live store:

```sh
set -a; . ~/.rusty_tick/tick.env; set +a
rusty-tick-restore-drill                      # newest local archive
rusty-tick-restore-drill /path/to/rusty-tick-YYYYMMDDTHHMMSSZ.tar.gz   # e.g. one fetched back from off-host
```

It checks the sha256 and unpacks into a temporary directory. There it serves the copy on
`127.0.0.1:18788` (`RUSTY_TICK_DRILL_PORT`) and counts each enabled user's tasks through the API.
It needs a throwaway token for each user, which it mints in the temporary copy only and passes to
curl from a file. It fails on a damaged archive, a busy port, a server that does not start, any
user whose tasks cannot be read, or zero tasks.

**Restore for real.** Stop mill-board and rusty-tick. Move the damaged `~/.rusty_tick/data`
aside; never delete it before the restore works. Then
`tar -xzf <archive> -C ~/.rusty_tick` and start rusty-tick, then mill-board.

Board settings (`~/.mill-board/settings`) and the env files are not in these archives. Keep a
copy of them off the host as well; the env files hold tokens.

Upgrade the binary and the web UI together, keeping the previous pair for rollback. Rebuild both as
in Install, then swap whole directories (`cp -r dist web` onto an existing `web` would nest `dist`
inside it):

```sh
sudo cp -p /usr/local/bin/rusty_tick /usr/local/bin/rusty_tick.prev
rm -rf ~/.rusty_tick/web.prev ~/.rusty_tick/web.new
cp -r crates/apps/rusty_tick/web/dist ~/.rusty_tick/web.new
sudo systemctl stop rusty-tick
sudo install -m 755 target/release/rusty_tick /usr/local/bin/
mv ~/.rusty_tick/web ~/.rusty_tick/web.prev && mv ~/.rusty_tick/web.new ~/.rusty_tick/web
sudo systemctl start rusty-tick
```

Roll back by stopping the service, putting `rusty_tick.prev` and `web.prev` back in place, and
starting it.
