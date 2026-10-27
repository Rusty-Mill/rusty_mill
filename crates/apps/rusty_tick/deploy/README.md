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
- Grants are kept in `<data-dir>/oauth.json` and checked against `users.json` on every request,
  so disabling a user ends their grants. Revoking one of their tokens, or changing
  `RUSTY_TICK_TOKEN`, does not. To end every grant, stop the server, delete `oauth.json` and
  start it again.
- Back up `oauth.json` with the rest of the data directory, or every connector signs in again.

## Backups, upgrade and rollback

Back up `~/.rusty_tick/data` with the service stopped, and copy it off the host.

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
