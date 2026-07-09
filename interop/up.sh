#!/usr/bin/env bash
# Bring up the local interop environment:
#   - Headscale 0.26 in Docker (host network, 127.0.0.1:8080)
#   - Two official tailscaled nodes (userspace networking, no root TUN)
#     joined to the local Headscale with preauth keys.
#
# Prereqs: docker with the headscale/headscale:0.26 image available, and the
# official tailscale static bundle extracted somewhere; pass its directory as
# TS_BIN_DIR (must contain `tailscaled` and `tailscale`).
#
# State lives under interop/state/ (gitignored). Idempotent-ish: `down.sh`
# first if re-running from scratch.
set -euo pipefail

cd "$(dirname "$0")"
TS_BIN_DIR="${TS_BIN_DIR:?set TS_BIN_DIR to the dir containing tailscaled/tailscale}"
STATE="$PWD/state"
mkdir -p "$STATE/headscale" "$STATE/ts1" "$STATE/ts2"

echo "== starting headscale =="
docker rm -f ts-rs-headscale >/dev/null 2>&1 || true
docker run -d --name ts-rs-headscale --network host \
  -v "$PWD/headscale:/etc/headscale:ro" \
  -v "$STATE/headscale:/var/lib/headscale" \
  headscale/headscale:0.26 serve

for i in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:8080/health >/dev/null 2>&1; then break; fi
  sleep 1
done
curl -fsS http://127.0.0.1:8080/health && echo " headscale healthy"

echo "== creating user + preauth keys =="
docker exec ts-rs-headscale headscale users create interop || true
KEY1=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h | tail -1)
KEY2=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h | tail -1)

start_node() {
  local n="$1" key="$2" port="$3"
  echo "== starting tailscaled node$n =="
  "$TS_BIN_DIR/tailscaled" \
    --tun=userspace-networking \
    --statedir="$STATE/ts$n" \
    --socket="$STATE/ts$n/tailscaled.sock" \
    --port="$port" \
    >"$STATE/ts$n/tailscaled.log" 2>&1 &
  echo $! > "$STATE/ts$n/tailscaled.pid"
  sleep 2
  "$TS_BIN_DIR/tailscale" --socket="$STATE/ts$n/tailscaled.sock" up \
    --login-server=http://127.0.0.1:8080 \
    --auth-key="$key" \
    --hostname="node$n"
}

start_node 1 "$KEY1" 41641
start_node 2 "$KEY2" 41642

echo "== tailnet status (node1) =="
"$TS_BIN_DIR/tailscale" --socket="$STATE/ts1/tailscaled.sock" status
echo
echo "LocalAPI sockets:"
echo "  node1: $STATE/ts1/tailscaled.sock"
echo "  node2: $STATE/ts2/tailscaled.sock"
