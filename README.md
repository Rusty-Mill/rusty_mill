# tailscale-rs (rusty_tail)

A sovereign, pure-Rust Tailscale client: control-plane client (ts2021),
WireGuard data plane, NAT traversal, and an embeddable library — no Go
binaries at runtime. Targets [Headscale](https://github.com/juanfont/headscale)
first; the hosted control plane is a later milestone.

**Status: Phase 5** — direct paths. On top of the Phase-4 TUN data plane,
two `ts-daemon` nodes now discover each other's endpoints (local + STUN
reflexive), run **disco** (ping/pong/call-me-maybe) over DERP and UDP, and
**upgrade from the DERP relay to a direct UDP path** — migrating the live
WireGuard session without dropping it. Verified end-to-end: two namespaced
nodes upgrade DERP→direct and carry real TCP over the direct path. See
[DESIGN.md](DESIGN.md) for the phase plan and every decision, and
[PROTOCOL.md](PROTOCOL.md) for the ts2021 + DERP + TUN + disco/STUN notes.

## Layout

Cargo workspace, one crate per subsystem under [`crates/`](crates/). Live:
`ts-types` (wire/API types), `ts-key` (keys), `ts-control` (ts2021 Noise IK
+ register + netmap), `ts-derp` (DERP relay client), `ts-wg` (boringtun
WireGuard adapter), `ts-tun` (TUN device + MagicDNS), `ts-stun` (STUN
client), `ts-disco` (disco codec), `ts-magicsock` (direct/DERP path mux),
`ts-engine` (control → WG → magicsock → TUN orchestration), `ts-cli`
(LocalAPI CLI), `ts-daemon` (the daemon). Placeholders filling in by phase:
`ts-filter`, `ts-localapi`, `ts-net`.

## Build & test

```console
$ cargo build --workspace
$ cargo test --workspace
$ cargo clippy --workspace --all-targets
```

## Try it against a real tailscaled

`ts-cli` talks to the LocalAPI Unix socket of any running `tailscaled`
(official or, eventually, our own `ts-daemon`):

```console
$ cargo run -p ts-cli -- status
$ cargo run -p ts-cli -- status --json
$ cargo run -p ts-cli -- ping 100.64.0.2
$ cargo run -p ts-cli -- down
$ cargo run -p ts-cli -- up
```

The socket defaults to `/var/run/tailscale/tailscaled.sock`; override with
`--socket <path>`.

## Register against Headscale with our own daemon

Bring up a local Headscale (`interop/up.sh` starts one plus two official
nodes), mint a preauth key, then join with the pure-Rust daemon:

```console
$ KEY=$(docker exec ts-rs-headscale headscale preauthkeys create --user 1 --expiration 24h | tail -1)
$ cargo run -p ts-daemon -- \
    --login-server http://127.0.0.1:8080 --authkey "$KEY" --hostname rusty-node
registered: node_key=nodekey:… hostname=rusty-node authorized=true
self:  100.64.0.3      rusty-node.tailnet.test. (node 3)
netmap: 2 peer(s)
  peer: 100.64.0.1      node1.tailnet.test.      offline
  peer: 100.64.0.2      node2.tailnet.test.      offline
```

## Ping another node over the DERP relay (Phase 3)

Start one node's data plane, then ping it from a second node — all traffic
is WireGuard tunnelled over DERP, no direct path, no TUN:

```console
# node A: register + run the data plane, stay up
$ cargo run -p ts-daemon -- --login-server http://127.0.0.1:8080 \
    --authkey "$KEY_A" --state-dir /tmp/a --hostname rusty-a
ts-daemon: data plane up (DERP-only). Ctrl-C to stop.
INFO engine: local tailnet address ip=100.64.0.4

# node B: register, then ping A's tailnet IP over the relay
$ cargo run -p ts-daemon -- --login-server http://127.0.0.1:8080 \
    --authkey "$KEY_B" --state-dir /tmp/b --hostname rusty-b --ping 100.64.0.4
pong from 100.64.0.4 via DERP relay in 6.4ms
```

The daemon in the default (no `--ping`) mode registers, streams the netmap,
and serves as a pingable node.

## Real apps across the tailnet with a TUN device (Phase 4)

With `--tun`, the daemon brings up a real `100.64/10` TUN interface, so the
OS routes normal traffic across the tailnet (relayed via DERP):

```console
$ sudo cargo run -p ts-daemon -- --login-server http://10.0.0.1:8080 \
    --authkey "$KEY" --tun ts0 --hosts-file /etc/hosts
INFO engine: TUN device up device=ts0 ip=100.64.0.14

# from another node, real tools just work:
$ ping 100.64.0.14
$ curl http://rusty-a.tailnet.test:8000     # name via MagicDNS hosts stub
```

The [`interop/tun_netns_test.sh`](interop/tun_netns_test.sh) harness stands
up two namespaced daemons and proves `ping` + `curl` across the tailnet over
the relay end-to-end. TUN mode needs `CAP_NET_ADMIN`; the userspace
(`--ping`) mode above needs no privileges.

## Direct paths (Phase 5)

Add `--direct` (and optionally `--stun host:port`) to enable direct-path
discovery: the daemon runs disco and upgrades peers from the DERP relay to a
direct UDP path when one is reachable, migrating the live WireGuard session.

```console
$ ts-daemon … --tun ts0 --direct --stun 10.0.0.1:3479
INFO magicsock: direct path UP (DERP→direct) peer=nodekey:… endpoint=10.0.0.20:34753
```

[`interop/direct_path_test.sh`](interop/direct_path_test.sh) stands up two
namespaced nodes and verifies they upgrade DERP→direct and carry TCP over
the direct path.

## Interop test environment

Until the `xtask` harness lands, the real-stack environment (Headscale in
Docker + two official tailscaled nodes in userspace mode) is brought up with
the scripts in [`interop/`](interop/) — see comments in
`interop/up.sh`.
