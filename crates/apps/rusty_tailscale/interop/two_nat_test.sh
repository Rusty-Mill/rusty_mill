#!/usr/bin/env bash
# Two-NAT direct-path test for the first-party mesh hub: a hub on a public
# segment and one `ts-net` node behind each of two Linux NATs. Needs root,
# iproute2 and iptables, on Linux. Fails (non-zero) unless the assertions hold.
#
#   two_nat_test.sh cone|symmetric [total bytes]
#
# cone       port-restricted cone: MASQUERADE keeps one external port per
#            internal socket. Asserts: the path moves from the relay to direct
#            within 10 s, and most of the bytes cross the wan as direct UDP.
# symmetric  MASQUERADE --random: a new external port per destination. Asserts:
#            the path never goes direct, almost no direct bytes cross the wan,
#            and the transfer still completes over the relay.
# Either way every byte arrives in order (the receiver checks the pattern).
#
# Build first (the script builds nothing):
#   cargo build -p ts-daemon -p ts-net --example mesh_node
# Override the binaries with TS_DAEMON and MESH_NODE.
set -euo pipefail
export PATH="$PATH:/usr/sbin:/sbin"

MODE=${1:?usage: two_nat_test.sh cone|symmetric [total bytes]}
TOTAL=${2:-12000000}
case "$MODE" in cone|symmetric) ;; *) echo "mode must be cone or symmetric" >&2; exit 2 ;; esac
[ "$(id -u)" = 0 ] || { echo "needs root (network namespaces)" >&2; exit 2; }

# Not `git rev-parse`: as root on a checkout another user owns, git refuses.
ROOT=$(cd "$(dirname "$0")/../../../.." && pwd)
TS_DAEMON=${TS_DAEMON:-$ROOT/target/debug/ts-daemon}
MESH_NODE=${MESH_NODE:-$ROOT/target/debug/examples/mesh_node}
for bin in "$TS_DAEMON" "$MESH_NODE"; do
  [ -x "$bin" ] || { echo "missing $bin (build it first)" >&2; exit 2; }
done

TAG=tsn$$
NS=("$TAG-hub" "$TAG-wan" "$TAG-natA" "$TAG-natB" "$TAG-a" "$TAG-b")
WORK=$(mktemp -d)
PIDS=()

cleanup() {
  local pid ns
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  for ns in "${NS[@]}"; do ip netns del "$ns" 2>/dev/null || true; done
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
in_ns() { local ns=$1; shift; ip netns exec "$TAG-$ns" "$@"; }

for ns in "${NS[@]}"; do ip netns add "$ns"; ip -n "$ns" link set lo up; done

link() { # link <ns1> <if1> <ip1> <ns2> <if2> <ip2>; interface names are per-namespace
  local tmp1="$TAG$2" tmp2="$TAG$5"
  ip link add "$tmp1" type veth peer name "$tmp2"
  ip link set "$tmp1" netns "$TAG-$1"; ip link set "$tmp2" netns "$TAG-$4"
  ip -n "$TAG-$1" link set "$tmp1" name "$2"; ip -n "$TAG-$4" link set "$tmp2" name "$5"
  ip -n "$TAG-$1" addr add "$3" dev "$2"; ip -n "$TAG-$4" addr add "$6" dev "$5"
  ip -n "$TAG-$1" link set "$2" up; ip -n "$TAG-$4" link set "$5" up
}
link hub h0 10.0.0.2/24 wan w0 10.0.0.1/24
link natA na0 10.0.1.2/24 wan wa 10.0.1.1/24
link natB nb0 10.0.2.2/24 wan wb 10.0.2.1/24
link natA la0 192.168.1.1/24 a a0 192.168.1.2/24
link natB lb0 192.168.2.1/24 b b0 192.168.2.2/24
ip -n "$TAG-hub" route add default via 10.0.0.1
ip -n "$TAG-natA" route add default via 10.0.1.1
ip -n "$TAG-natB" route add default via 10.0.2.1
ip -n "$TAG-a" route add default via 192.168.1.1
ip -n "$TAG-b" route add default via 192.168.2.1
for n in wan natA natB; do in_ns "$n" sysctl -qw net.ipv4.ip_forward=1; done

# Count the direct UDP the wan forwards between the two NATs (rules with no
# target only count).
in_ns wan iptables -A FORWARD -s 10.0.1.2 -d 10.0.2.2 -p udp
in_ns wan iptables -A FORWARD -s 10.0.2.2 -d 10.0.1.2 -p udp

# An unsolicited packet for the NAT box itself must not leave a conntrack
# entry: it would take the tuple the node's own next outbound packet needs and
# force a new external port (an accidental symmetric NAT, and direct paths
# never form). Dropping it in INPUT keeps it unconfirmed.
in_ns natA iptables -A INPUT -i na0 -p udp -j DROP
in_ns natB iptables -A INPUT -i nb0 -p udp -j DROP
RANDOM_PORTS=()
[ "$MODE" = symmetric ] && RANDOM_PORTS=(--random)
in_ns natA iptables -t nat -A POSTROUTING -o na0 -j MASQUERADE "${RANDOM_PORTS[@]}"
in_ns natB iptables -t nat -A POSTROUTING -o nb0 -j MASQUERADE "${RANDOM_PORTS[@]}"

# The hub: pre-auth key and allow-all policy; STUN shares the control port.
printf 'twonatkey test@example.com reusable\n' >"$WORK/keys"; chmod 600 "$WORK/keys"
printf '* -> *\n' >"$WORK/policy"
in_ns hub "$TS_DAEMON" hub --listen 10.0.0.2:8080 --stun 10.0.0.2:8080 --advertise 10.0.0.2 \
  --state-dir "$WORK/hub-state" --keys "$WORK/keys" --policy "$WORK/policy" >"$WORK/hub.log" 2>&1 &
PIDS+=($!)
wait_for() { # wait_for <file> <pattern> <seconds> <what>
  local i
  for ((i = 0; i < $3 * 5; i++)); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; done
  cat "$1" >&2 || true
  fail "$4 did not appear within $3 s"
}
wait_for "$WORK/hub.log" "hub: listening on" 10 "the hub"

URL=http://10.0.0.2:8080
in_ns b "$MESH_NODE" serve "$URL" 10.0.0.2:8080 twonatkey srv 7000 "$TOTAL" >"$WORK/srv.log" 2>&1 &
SRV=$!; PIDS+=("$SRV")
wait_for "$WORK/srv.log" READY 30 "the server node"
PEER=$(awk '/^READY/{print $2}' "$WORK/srv.log")
echo "mode=$MODE server tailnet ip=$PEER"

in_ns a "$MESH_NODE" send "$URL" 10.0.0.2:8080 twonatkey cli "$PEER" 7000 "$TOTAL" >"$WORK/cli.log" 2>&1 &
CLI=$!; PIDS+=("$CLI")

# The receiver exits when it has every byte (or fails); bound the wait.
deadline=$((SECONDS + 120))
while kill -0 "$SRV" 2>/dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || { cat "$WORK/srv.log" "$WORK/cli.log" >&2; fail "transfer did not finish in 120 s"; }
  sleep 0.5
done
wait "$SRV" || { cat "$WORK/srv.log" "$WORK/cli.log" >&2; fail "the receiver reported an error"; }
grep -q "^DONE recv $TOTAL ok" "$WORK/srv.log" || { cat "$WORK/srv.log" >&2; fail "receiver did not confirm $TOTAL bytes"; }
wait_for "$WORK/cli.log" "^DONE sent $TOTAL" 30 "the sender's summary"

FIRST=$(sed -n 's/^DONE sent [0-9]* first_direct=//p' "$WORK/cli.log")
# Bytes the wan forwarded NAT to NAT: direct UDP, not through the hub.
DIRECT=$(in_ns wan iptables -L FORWARD -v -n -x | awk '$7 ~ /^10\.0\.[12]\.2$/ {sum += $2} END {print sum + 0}')
echo "first_direct=$FIRST direct_wan_bytes=$DIRECT total=$TOTAL"
cat "$WORK/cli.log"

case "$MODE" in
  cone)
    [[ "$FIRST" =~ ^[0-9]+$ ]] || fail "cone NAT: the path never went direct"
    [ "$FIRST" -le 10 ] || fail "cone NAT: the path went direct after $FIRST s (want within 10 s)"
    [ "$DIRECT" -ge $((TOTAL / 2)) ] || fail "cone NAT: only $DIRECT of $TOTAL bytes went direct"
    ;;
  symmetric)
    [ "$FIRST" = never ] || fail "symmetric NAT: the path went direct after $FIRST s (want relay only)"
    [ "$DIRECT" -lt $((TOTAL / 10)) ] || fail "symmetric NAT: $DIRECT bytes crossed as direct UDP (want almost none)"
    ;;
esac
echo "PASS $MODE"
