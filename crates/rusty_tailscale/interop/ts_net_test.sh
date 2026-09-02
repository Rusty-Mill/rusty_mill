#!/usr/bin/env bash
# Phase-7 end-to-end verification: an HTTP server built on ts-net — a fully
# userspace TCP/IP stack (smoltcp), NO TUN device and NO root inside the app —
# is reachable across the tailnet from another node.
#
#   ns-a (10.0.0.10)  --\                    /--  ns-b (10.0.0.20)
#     ts0 100.64.0.a     >-- br-ts 10.0.0.1 --<     (no TUN!)
#     ts-daemon --tun ---/   (host: Headscale  \--  ts-net serve_http :8080
#     curls ns-b:8080         + embedded DERP)       userspace smoltcp stack
#
# ns-b runs the ts-net example (userspace stack, no TUN); ns-a uses a real TUN
# so an ordinary `curl` originates from the kernel and rides the tailnet to the
# ts-net server, relayed via DERP.
#
# Requires: root/CAP_NET_ADMIN, iproute2, a running Headscale on the host at
# http://10.0.0.1:8080, the built ts-daemon + ts-net example, and curl.
set -uo pipefail
export PATH="$PATH:/usr/sbin:/sbin"

DAEMON="${TS_DAEMON:-/home/user/rusty_tail/target/debug/ts-daemon}"
SERVER="${TS_NET_SERVER:-/home/user/rusty_tail/target/debug/examples/serve_http}"
CTRL="http://10.0.0.1:8080"
SCR="${SCRATCH:-/tmp/tsrs-phase7}"
mkdir -p "$SCR"

cleanup() {
  ip netns pids ns-a 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns pids ns-b 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns del ns-a 2>/dev/null
  ip netns del ns-b 2>/dev/null
  ip link del v-ns-a 2>/dev/null
  ip link del v-ns-b 2>/dev/null
  ip link del br-ts 2>/dev/null
}
trap cleanup EXIT
cleanup

echo "== bridge + namespaces =="
ip link add br-ts type bridge
ip addr add 10.0.0.1/24 dev br-ts
ip link set br-ts up

setup_ns() {
  local ns="$1" nsip="$2"
  ip netns add "$ns"
  ip link add "v-$ns" type veth peer name "v-$ns-p"
  ip link set "v-$ns" master br-ts
  ip link set "v-$ns" up
  ip link set "v-$ns-p" netns "$ns"
  ip -n "$ns" addr add "$nsip/24" dev "v-$ns-p"
  ip -n "$ns" link set "v-$ns-p" up
  ip -n "$ns" link set lo up
  ip -n "$ns" route add default via 10.0.0.1
}
setup_ns ns-a 10.0.0.10
setup_ns ns-b 10.0.0.20

echo "== reachability check =="
ip netns exec ns-a curl -fsS --max-time 5 "$CTRL/health" >/dev/null \
  && echo "  ns-a can reach Headscale" || { echo "  FAIL: ns-a cannot reach $CTRL"; exit 1; }

echo "== preauth keys =="
KA=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h 2>/dev/null | tail -1)
KB=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h 2>/dev/null | tail -1)

echo "== ns-b: ts-net HTTP server (userspace stack, NO TUN) =="
rm -rf "$SCR/ns-b"
ip netns exec ns-b env RUST_LOG=info "$SERVER" \
  --login-server "$CTRL" --authkey "$KB" \
  --state-dir "$SCR/ns-b" --hostname rusty-web --port 8080 \
  >"$SCR/ns-b.log" 2>&1 &
echo $! >"$SCR/ns-b.pid"

# The example prints "serving http://<ip>:8080/" once it has its tailnet IP.
IP_B=""
for _ in $(seq 1 80); do
  IP_B=$(grep -oE "http://100\.64\.[0-9]+\.[0-9]+:8080" "$SCR/ns-b.log" 2>/dev/null | grep -oE "100\.64\.[0-9]+\.[0-9]+" | head -1)
  [ -n "$IP_B" ] && break
  sleep 0.5
done
[ -n "$IP_B" ] || { echo "FAIL: ts-net server never announced a tailnet IP"; sed 's/^/    /' "$SCR/ns-b.log"; exit 1; }
echo "  ts-net server tailnet IP: $IP_B"

echo "== ns-a: ts-daemon with a real TUN (the client) =="
rm -rf "$SCR/ns-a"
ip netns exec ns-a env RUST_LOG=info "$DAEMON" \
  --login-server "$CTRL" --derp-server "$CTRL" \
  --authkey "$KA" --state-dir "$SCR/ns-a" --hostname rusty-client \
  --tun ts0 >"$SCR/ns-a.log" 2>&1 &
echo $! >"$SCR/ns-a.pid"

get_tun_ip() {
  for _ in $(seq 1 40); do
    local ip
    ip=$(ip -n ns-a -4 addr show ts0 2>/dev/null | grep -oE "100\.64\.[0-9]+\.[0-9]+" | head -1)
    [ -n "$ip" ] && { echo "$ip"; return 0; }
    sleep 0.5
  done
  return 1
}
IP_A=$(get_tun_ip) || { echo "FAIL: ns-a TUN never came up"; sed 's/^/    /' "$SCR/ns-a.log"; exit 1; }
echo "  ns-a (client) tailnet IP: $IP_A"

echo "== curl ns-a -> ts-net server http://$IP_B:8080/ (userspace stack, over DERP) =="
BODY=""
for _ in $(seq 1 60); do
  BODY=$(ip netns exec ns-a curl -s --max-time 5 "http://$IP_B:8080/" 2>/dev/null)
  [ -n "$BODY" ] && break
  sleep 0.5
done
echo "  response: $BODY"

echo
if echo "$BODY" | grep -q "tailscale-rs (ts-net)"; then
  echo "RESULT: PASS — an HTTP server on ts-net (no TUN, no root) is reachable across the tailnet"
  RC=0
else
  echo "RESULT: FAIL — curl did not return the ts-net server's page"
  echo "--- ns-b (ts-net server) log tail ---"; tail -15 "$SCR/ns-b.log" | sed 's/^/    /'
  echo "--- ns-a (client) log tail ---"; tail -15 "$SCR/ns-a.log" | sed 's/^/    /'
  echo "--- curl -v ---"
  ip netns exec ns-a curl -sv --max-time 10 "http://$IP_B:8080/" 2>&1 | grep -vE "no_proxy|Uses proxy" | head -15
  RC=1
fi
[ "${KEEP:-0}" = 1 ] && { echo "(KEEP=1: leaving namespaces up)"; trap - EXIT; }
exit $RC
