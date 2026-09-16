#!/usr/bin/env bash
# Phase-5 verification: two ts-daemon nodes with direct-path discovery
# (magicsock/disco) enabled. They register with Headscale, connect to DERP,
# exchange call-me-maybe, disco-ping each other's endpoints, and UPGRADE from
# the DERP relay to a direct UDP path — while the WireGuard session keeps
# running. Then real traffic (curl) flows over the direct path.
#
# Topology mirrors the Phase-4 harness (bridge + two netns). On this flat L2
# the nodes' local endpoints (10.0.0.x) are directly reachable, so a direct
# path is expected; NAT traversal (STUN + hole punching) is exercised
# separately.
set -uo pipefail
export PATH="$PATH:/usr/sbin:/sbin"

BIN="${TS_DAEMON:-/home/user/rusty_tail/target/debug/ts-daemon}"
CTRL="http://10.0.0.1:8080"
SCR="${SCRATCH:-/tmp/tsrs-phase5}"
mkdir -p "$SCR"

cleanup() {
  ip netns pids ns-a 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns pids ns-b 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns del ns-a 2>/dev/null; ip netns del ns-b 2>/dev/null
  ip link del v-ns-a 2>/dev/null; ip link del v-ns-b 2>/dev/null
  iptables -D FORWARD -i br-ts -o br-ts -j ACCEPT 2>/dev/null
  ip link del br-ts 2>/dev/null
}
trap cleanup EXIT
cleanup

echo "== bridge + namespaces =="
ip link add br-ts type bridge; ip addr add 10.0.0.1/24 dev br-ts; ip link set br-ts up
# Docker sets the iptables FORWARD policy to DROP and br_netfilter routes
# bridged traffic through it, which would block the direct node↔node UDP path.
# Allow same-bridge forwarding so disco/direct paths can actually work.
iptables -I FORWARD -i br-ts -o br-ts -j ACCEPT
setup_ns() {
  local ns="$1" nsip="$2"
  ip netns add "$ns"
  ip link add "v-$ns" type veth peer name "p-$ns"
  ip link set "v-$ns" master br-ts; ip link set "v-$ns" up
  ip link set "p-$ns" netns "$ns"
  ip -n "$ns" addr add "$nsip/24" dev "p-$ns"; ip -n "$ns" link set "p-$ns" up
  ip -n "$ns" link set lo up; ip -n "$ns" route add default via 10.0.0.1
}
setup_ns ns-a 10.0.0.10
setup_ns ns-b 10.0.0.20

ip netns exec ns-a curl -fsS --max-time 5 "$CTRL/health" >/dev/null \
  && echo "  ns-a can reach Headscale" || { echo "FAIL: no Headscale"; exit 1; }

echo "== STUN server on 10.0.0.1:3479 =="
python3 "$(dirname "$0")/stun_server.py" 10.0.0.1 3478 >"$SCR/stun.log" 2>&1 &
STUN_PID=$!
trap 'kill $STUN_PID 2>/dev/null; cleanup' EXIT
sleep 0.5

KA=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h 2>/dev/null | tail -1)
KB=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h 2>/dev/null | tail -1)

start() {
  local ns="$1" key="$2" name="$3"
  rm -rf "$SCR/$ns"
  ip netns exec "$ns" env RUST_LOG=info,ts_magicsock=info "$BIN" \
    --login-server "$CTRL" --derp-server "$CTRL" \
    --authkey "$key" --state-dir "$SCR/$ns" --hostname "$name" \
    --tun ts0 --direct --stun 10.0.0.1:3479 >"$SCR/$ns.log" 2>&1 &
}
echo "== starting daemons (--direct) =="
start ns-a "$KA" rusty-a
start ns-b "$KB" rusty-b

get_ip() { for _ in $(seq 1 40); do local i; i=$(ip -n "$1" -4 addr show ts0 2>/dev/null | grep -oE "100\.64\.[0-9.]+" | head -1); [ -n "$i" ] && { echo "$i"; return; }; sleep 0.5; done; }
IP_A=$(get_ip ns-a); IP_B=$(get_ip ns-b)
echo "  ns-a=$IP_A  ns-b=$IP_B"

echo "== waiting for DERP→direct upgrade =="
UP_A=""; UP_B=""
for _ in $(seq 1 90); do
  UP_A=$(grep -oE "direct path UP.*" "$SCR/ns-a.log" 2>/dev/null | head -1)
  UP_B=$(grep -oE "direct path UP.*" "$SCR/ns-b.log" 2>/dev/null | head -1)
  [ -n "$UP_A" ] && [ -n "$UP_B" ] && break
  sleep 0.5
done
echo "  ns-a: ${UP_A:-<none>}"
echo "  ns-b: ${UP_B:-<none>}"

echo "== data over the direct path: curl ns-a -> ns-b =="
echo "hello over direct path" > "$SCR/index.html"
( cd "$SCR" && ip netns exec ns-b python3 -m http.server 8000 --bind "$IP_B" >/dev/null 2>&1 ) &
for _ in $(seq 1 20); do ip netns exec ns-b curl -s --max-time 2 "http://$IP_B:8000/index.html" >/dev/null 2>&1 && break; sleep 0.5; done
BODY=$(ip netns exec ns-a curl -s --max-time 12 "http://$IP_B:8000/index.html")
echo "  response: $BODY"

echo
if [ -n "$UP_A" ] && [ -n "$UP_B" ] && [ "$BODY" = "hello over direct path" ]; then
  echo "RESULT: PASS — both nodes upgraded DERP→direct and traffic flows over it"
  RC=0
else
  echo "RESULT: FAIL"
  echo "--- ns-a log (disco/path) ---"; grep -iE "magicsock|direct|derp|tailnet" "$SCR/ns-a.log" | tail -12
  RC=1
fi
[ "${KEEP:-0}" = 1 ] && trap - EXIT
exit $RC
