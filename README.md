# tailscale-rs (rusty_tail)

A sovereign, pure-Rust Tailscale client: control-plane client (ts2021),
WireGuard data plane, NAT traversal, and an embeddable library — no Go
binaries at runtime. Targets [Headscale](https://github.com/juanfont/headscale)
first; the hosted control plane is a later milestone.

**Status: Phase 1** — `ts-cli` speaks LocalAPI to a real `tailscaled`.
See [DESIGN.md](DESIGN.md) for the full phase plan and every design decision.

## Layout

Cargo workspace, one crate per subsystem under [`crates/`](crates/):
`ts-types` (wire/API types) and `ts-cli` (LocalAPI CLI) are live; the rest
(`ts-control`, `ts-derp`, `ts-magicsock`, `ts-wg`, …) are placeholders that
fill in phase by phase.

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

## Interop test environment

Until the `xtask` harness lands, the real-stack environment (Headscale in
Docker + two official tailscaled nodes in userspace mode) is brought up with
the scripts in [`interop/`](interop/) — see comments in
`interop/up.sh`.
