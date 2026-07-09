# tailscale-rs (rusty_tail)

A sovereign, pure-Rust Tailscale client: control-plane client (ts2021),
WireGuard data plane, NAT traversal, and an embeddable library — no Go
binaries at runtime. Targets [Headscale](https://github.com/juanfont/headscale)
first; the hosted control plane is a later milestone.

**Status: Phase 7** — embeddable node + hardening. On top of the Phase-6
daemon, `ts-net` provides a **fully userspace TCP/IP stack (smoltcp) on the
tailnet — no TUN device and no root**. An application `bind`s a port on its
tailnet IP and serves connections as ordinary `tokio` streams, so a plain
`cargo run` becomes a tailnet service. Verified end-to-end: the
[`serve_http` example](crates/ts-net/examples/serve_http.rs) serves a page
that another node fetches with `curl`, entirely in userspace. The wire
parsers gain randomized panic-free fuzz-smoke harnesses. See
[DESIGN.md](DESIGN.md) for the phase plan and every decision, and
[PROTOCOL.md](PROTOCOL.md) for the ts2021 + DERP + TUN + disco/STUN + ACL +
netstack notes.

## Layout

Cargo workspace, one crate per subsystem under [`crates/`](crates/). Live:
`ts-types` (wire/API types), `ts-key` (keys), `ts-control` (ts2021 Noise IK
+ register + netmap), `ts-derp` (DERP relay client), `ts-wg` (boringtun
WireGuard adapter), `ts-tun` (TUN device + MagicDNS), `ts-stun` (STUN
client), `ts-disco` (disco codec), `ts-magicsock` (direct/DERP path mux),
`ts-filter` (netmap ACL enforcement), `ts-engine` (control → WG → magicsock
→ TUN orchestration), `ts-localapi` (HTTP-over-UDS LocalAPI server), `ts-cli`
(LocalAPI CLI), `ts-daemon` (the daemon), `ts-net` (embeddable userspace
netstack over smoltcp). All 15 crates are now live.

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

## Drive the daemon with `ts-cli` (Phase 6)

`ts-daemon` serves the LocalAPI on a Unix socket (`--socket`, default
`/var/run/tailscale/tailscaled.sock`), so the Phase-1 CLI talks to our own
daemon exactly as it does to `tailscaled`:

```console
$ ts-daemon … --socket /run/tailscale/tailscaled.sock &
$ ts-cli --socket /run/tailscale/tailscaled.sock status
100.64.0.41     rusty-localapi   -          linux   -
100.64.0.19     rusty-a          interop@   linux   active; relay "derp"
$ ts-cli --socket … down     # WantRunning=false: traffic stops, tunnel survives
$ ts-cli --socket … up
$ ts-cli --socket … ping 100.64.0.19
pong from rusty-a (100.64.0.19) via DERP(derp) in 1ms
```

Inbound traffic is checked against the tailnet **ACL** (the netmap packet
filter): with a restrictive policy the daemon drops packets no rule permits.

## Embed a tailnet service — no TUN, no root (Phase 7)

`ts-net` runs a userspace TCP/IP stack (smoltcp) fed directly by the engine's
decrypted WireGuard packets, so an application serves on the tailnet without a
TUN device or any privileges:

```rust
let node = ts_net::Node::new(config).await?;
let ip = node.wait_ip().await.unwrap();          // our 100.64.x.y
let mut listener = node.bind(8080).await?;        // TCP listener on the tailnet
while let Some(mut stream) = listener.accept().await {
    // stream: AsyncRead + AsyncWrite, like a std TcpStream
    tokio::spawn(async move { /* serve HTTP, etc. */ });
}
```

The [`serve_http` example](crates/ts-net/examples/serve_http.rs) is a complete
tailnet web server:

```console
$ cargo run -p ts-net --example serve_http -- \
    --login-server http://127.0.0.1:8080 --authkey "$KEY" --hostname rusty-web
ts-net: serving http://100.64.0.7:8080/ on the tailnet (no TUN, no root)

# from any other node on the tailnet:
$ curl http://100.64.0.7:8080/
Hello from tailscale-rs (ts-net), served with no TUN and no root!
```

[`interop/ts_net_test.sh`](interop/ts_net_test.sh) stands up the ts-net server
in one namespace and a TUN client in another, and proves a `curl` reaches the
userspace server across the tailnet.

## Run under systemd

[`packaging/tailscale-rs.service`](packaging/tailscale-rs.service) is a
hardened unit (`CAP_NET_ADMIN` only, `NoNewPrivileges`, `ProtectSystem`); it
reads the auth key from `/etc/tailscale-rs/env` so it never lands in the unit
file or the process table. `ts-daemon` shuts down cleanly on `SIGTERM`.

## Interop test environment

Until the `xtask` harness lands, the real-stack environment (Headscale in
Docker + two official tailscaled nodes in userspace mode) is brought up with
the scripts in [`interop/`](interop/) — see comments in
`interop/up.sh`.
