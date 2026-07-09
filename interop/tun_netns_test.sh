#!/usr/bin/env bash
# Phase-4 end-to-end verification: two ts-daemon nodes, each with a real TUN
# device in its own network namespace, joined to Headscale, exchanging real
# TCP/ICMP across the tailnet — relayed via DERP (no direct path yet).
#
#   ns-a (10.0.0.10) --\                     /-- ns-b (10.0.0.20)
#     ts0 100.64.0.a    >-- br-ts 10.0.0.1 --<     ts0 100.64.0.b
#     ts-daemon --------/   (host: Headscale   \-- ts-daemon
#                            + embedded DERP)       + python http.server
#
# Requires: root/CAP_NET_ADMIN, iproute2, a running Headscale on the host
# reachable at http://10.0.0.1:8080 (see server_url note below), the built
# ts-daemon binary, and python3.
set -uo pipefail
export PATH="$PATH:/usr/sbin:/sbin"

BIN="${TS_DAEMON:-/home/user/rusty_tail/target/debug/ts-daemon}"
CTRL="http://10.0.0.1:8080"
SCR="${SCRATCH:-/tmp/tsrs-phase4}"
mkdir -p "$SCR"

cleanup() {
  ip netns pids ns-a 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns pids ns-b 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns del ns-a 2>/dev/null
  ip netns del ns-b 2>/dev/null
  # Host-side veths outlive the bridge; delete them so re-runs don't collide.
  ip link del v-ns-a 2>/dev/null
  ip link del v-ns-b 2>/dev/null
  ip link del br-ts 2>/dev/null
}
trap cleanup EXIT
cleanup  # start clean

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

start_daemon() {
  local ns="$1" key="$2" name="$3"
  rm -rf "$SCR/$ns"
  ip netns exec "$ns" env RUST_LOG=info "$BIN" \
    --login-server "$CTRL" --derp-server "$CTRL" \
    --authkey "$key" --state-dir "$SCR/$ns" --hostname "$name" \
    --tun ts0 --hosts-file "$SCR/$ns/hosts" \
    >"$SCR/$ns.log" 2>&1 &
  echo $! >"$SCR/$ns.pid"
}

echo "== starting daemons =="
start_daemon ns-a "$KA" rusty-a
start_daemon ns-b "$KB" rusty-b

get_tun_ip() {
  local ns="$1"
  for _ in $(seq 1 40); do
    local ip
    ip=$(ip -n "$ns" -4 addr show ts0 2>/dev/null | grep -oE "100\.64\.[0-9]+\.[0-9]+" | head -1)
    [ -n "$ip" ] && { echo "$ip"; return 0; }
    sleep 0.5
  done
  return 1
}
IP_A=$(get_tun_ip ns-a) || { echo "FAIL: ns-a TUN never came up"; cat "$SCR/ns-a.log"; exit 1; }
IP_B=$(get_tun_ip ns-b) || { echo "FAIL: ns-b TUN never came up"; cat "$SCR/ns-b.log"; exit 1; }
echo "  ns-a tailnet IP: $IP_A"
echo "  ns-b tailnet IP: $IP_B"

echo "== waiting for netmaps to converge (each node learns the other) =="
converged=0
for _ in $(seq 1 40); do
  if grep -q "$IP_B" "$SCR/ns-a/hosts" 2>/dev/null && grep -q "$IP_A" "$SCR/ns-b/hosts" 2>/dev/null; then
    converged=1; break
  fi
  sleep 0.5
done
[ "$converged" = 1 ] && echo "  converged" || echo "  WARN: netmaps did not fully converge"

echo "== HTTP server in ns-b on $IP_B:8000 =="
echo "hello from rusty-b over the tailnet" > "$SCR/index.html"
( cd "$SCR" && ip netns exec ns-b python3 -m http.server 8000 --bind "$IP_B" >"$SCR/httpd.log" 2>&1 ) &
echo $! > "$SCR/httpd.pid"
# Wait until the server actually accepts a local connection.
for _ in $(seq 1 20); do
  if ip netns exec ns-b curl -s --max-time 2 "http://$IP_B:8000/index.html" >/dev/null 2>&1; then
    echo "  httpd listening"; break
  fi
  sleep 0.5
done
[ -s "$SCR/httpd.log" ] && { echo "  httpd stderr:"; sed 's/^/    /' "$SCR/httpd.log"; }

echo "== ping ns-a -> ns-b over DERP relay =="
ip netns exec ns-a ping -c 3 -W 5 "$IP_B" && PING_OK=1 || PING_OK=0

echo "== curl ns-a -> http://$IP_B:8000 over DERP relay =="
BODY=$(ip netns exec ns-a curl -s --max-time 15 "http://$IP_B:8000/index.html")
echo "  response: $BODY"

echo "== MagicDNS hosts written by ns-a =="
grep -A3 "BEGIN tailscale-rs" "$SCR/ns-a/hosts" 2>/dev/null || echo "  (no hosts block)"

echo
if [ "${BODY:-}" = "hello from rusty-b over the tailnet" ]; then
  echo "RESULT: PASS — TCP (curl) works across the tailnet via TUN, relayed"
  RC=0
else
  echo "RESULT: FAIL — curl did not return expected body"
  echo "--- httpd reachable locally in ns-b? ---"
  ip netns exec ns-b curl -s --max-time 5 "http://$IP_B:8000/index.html" || echo "  (local curl failed)"
  echo "--- curl -v ns-a -> ns-b ---"
  ip netns exec ns-a curl -sv --max-time 15 "http://$IP_B:8000/index.html" 2>&1 \
    | grep -vE "no_proxy|Uses proxy|^\{" | head -15
  RC=1
fi
[ "${KEEP:-0}" = 1 ] && { echo "(KEEP=1: leaving namespaces up)"; trap - EXIT; }
exit $RC
