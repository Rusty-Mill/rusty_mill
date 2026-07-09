# tailscale-rs (rusty_tail)

A sovereign, pure-Rust Tailscale client: control-plane client (ts2021),
WireGuard data plane, NAT traversal, and an embeddable library — no Go
binaries at runtime. Targets [Headscale](https://github.com/juanfont/headscale)
first; the hosted control plane is a later milestone.

**Status: Phase 2** — `ts-daemon` registers with a real Headscale server
over a pure-Rust ts2021 control channel (Noise IK + HTTP/2, no Go) and
streams the live netmap. `ts-cli` speaks LocalAPI to a real `tailscaled`.
No data plane yet (Phase 3). See [DESIGN.md](DESIGN.md) for the full phase
plan and every design decision, and [PROTOCOL.md](PROTOCOL.md) for the
ts2021 wire protocol notes.

## Layout

Cargo workspace, one crate per subsystem under [`crates/`](crates/):
`ts-types` (wire/API types), `ts-key` (key management), `ts-control`
(ts2021 Noise IK + register + netmap), `ts-cli` (LocalAPI CLI), and
`ts-daemon` (the daemon) are live; the rest (`ts-derp`, `ts-magicsock`,
`ts-wg`, …) are placeholders that fill in phase by phase.

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

The daemon keeps the netmap long-poll open and prints deltas
(`peer changed`, `peer removed`) as the tailnet changes. `--once` registers
and prints one netmap, then exits.

## Interop test environment

Until the `xtask` harness lands, the real-stack environment (Headscale in
Docker + two official tailscaled nodes in userspace mode) is brought up with
the scripts in [`interop/`](interop/) — see comments in
`interop/up.sh`.
