#!/usr/bin/env bash
# Spike C harness: a hub on a public segment, two nodes each behind its own
# Linux NAT. Needs root, iproute2 and iptables. Throwaway.
#
#   two-nat.sh cone|full|symmetric [total bytes]
#
# cone      port-restricted cone: MASQUERADE keeps one external port per
#           internal socket; inbound only from addresses it has sent to.
# full      full cone: as cone, and any inbound UDP is forwarded to the node.
# symmetric MASQUERADE --random: a new external port per destination.
set -euo pipefail
MODE=${1:?cone|full|symmetric}
TOTAL=${2:-20971520}
BIN=${BIN:-$(git rev-parse --show-toplevel)/target/release/examples}
CERT=${CERT:-/tmp/hubtls}
NS="hub wan natA natB a b"
cleanup() { for n in $NS; do ip netns del "$n" 2>/dev/null || true; done; pkill -f "examples/hub " 2>/dev/null || true; }
trap cleanup EXIT
cleanup
for n in $NS; do ip netns add "$n"; ip -n "$n" link set lo up; done

link() { # link <ns1> <if1> <ip1> <ns2> <if2> <ip2>
  ip link add "$2" type veth peer name "$5"
  ip link set "$2" netns "$1"; ip link set "$5" netns "$4"
  ip -n "$1" addr add "$3" dev "$2"; ip -n "$4" addr add "$6" dev "$5"
  ip -n "$1" link set "$2" up; ip -n "$4" link set "$5" up
}
link hub h0 10.0.0.2/24 wan w0 10.0.0.1/24
link natA na0 10.0.1.2/24 wan wa 10.0.1.1/24
link natB nb0 10.0.2.2/24 wan wb 10.0.2.1/24
link natA la0 192.168.1.1/24 a a0 192.168.1.2/24
link natB lb0 192.168.2.1/24 b b0 192.168.2.2/24
ip -n hub route add default via 10.0.0.1
ip -n natA route add default via 10.0.1.1
ip -n natB route add default via 10.0.2.1
ip -n a route add default via 192.168.1.1
ip -n b route add default via 192.168.2.1
for n in wan natA natB; do ip netns exec "$n" sysctl -qw net.ipv4.ip_forward=1; done
ip netns exec wan iptables -A FORWARD -s 10.0.1.2 -d 10.0.2.2 -p udp
ip netns exec wan iptables -A FORWARD -s 10.0.2.2 -d 10.0.1.2 -p udp
EXTRA=""; [ "$MODE" = symmetric ] && EXTRA="--random"
# An unsolicited packet for the NAT box itself must not leave a conntrack
# entry: it would take the tuple the node's own outbound packet needs next
# and force a new external port (an accidental symmetric NAT). Dropping it in
# INPUT keeps it unconfirmed.
ip netns exec natA iptables -A INPUT -i na0 -p udp -j DROP
ip netns exec natB iptables -A INPUT -i nb0 -p udp -j DROP
if [ "$MODE" = full ]; then
  ip netns exec natA iptables -t nat -A PREROUTING -i na0 -p udp -j DNAT --to-destination 192.168.1.2
  ip netns exec natB iptables -t nat -A PREROUTING -i nb0 -p udp -j DNAT --to-destination 192.168.2.2
fi
ip netns exec natA iptables -t nat -A POSTROUTING -o na0 -j MASQUERADE $EXTRA
ip netns exec natB iptables -t nat -A POSTROUTING -o nb0 -j MASQUERADE $EXTRA

HUB_TRACE=1 ip netns exec hub "$BIN/hub" 10.0.0.2:8080 10.0.0.2 spikekey "$CERT/c.pem" "$CERT/k.pem" >/tmp/spike-c-hub.log 2>&1 &
sleep 1
URL=http://10.0.0.2:8080
ip netns exec b "$BIN/mesh_node" serve "$URL" 10.0.0.2:8080 srv 7000 "$TOTAL" >/tmp/spike-c-srv.log 2>&1 &
SRV=$!
for _ in $(seq 50); do grep -q READY /tmp/spike-c-srv.log && break; sleep 0.2; done
grep -q READY /tmp/spike-c-srv.log || { cat /tmp/spike-c-srv.log; exit 1; }
PEER=$(awk '/READY/{print $2}' /tmp/spike-c-srv.log)
echo "mode=$MODE server tailnet ip=$PEER"
ip netns exec a "$BIN/mesh_node" send "$URL" 10.0.0.2:8080 cli "$PEER" 7000 "$TOTAL" | tee /tmp/spike-c-cli.log &
CLI=$!
wait $SRV || true
sleep 2; kill $CLI 2>/dev/null || true
echo "--- conntrack natA"; ip netns exec natA cat /proc/net/nf_conntrack 2>&1 | grep udp | head -8
echo "--- conntrack natB"; ip netns exec natB cat /proc/net/nf_conntrack 2>&1 | grep udp | head -8
echo "--- server"; cat /tmp/spike-c-srv.log
echo "--- direct UDP seen on the wan (A to B, B to A)"; ip netns exec wan iptables -L FORWARD -v -n -x | grep -E "10.0.[12].2"
echo "--- hub"; grep -v "^derp " /tmp/spike-c-hub.log | tail -20
