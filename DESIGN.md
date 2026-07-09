# tailscale-rs — Design Notes

A sovereign, pure-Rust Tailscale client: control-plane client, WireGuard data
plane, NAT traversal, and an embeddable library — with no dependency on the Go
binaries at runtime. This document records every strategic decision and every
deviation from the Go implementation's behavior, as they are made.

## Strategic decisions (fixed at project start)

1. **Pure Rust, end-to-end.** The learning/ownership path was explicitly chosen
   over FFI to `libtailscale`. No Go at runtime; the Go source is used as the
   protocol *specification* and the official binaries as interop *test peers*.
2. **Headscale first.** The open-source control server is the initial target;
   compatibility with Tailscale's hosted control plane is a later milestone,
   not a prerequisite.
3. **DERP-only data plane before NAT traversal.** A complete, correct (if slow)
   relayed transport ships first, isolating WireGuard/session bugs from
   path-discovery bugs. Direct paths are an upgrade, never a blocker.
4. **boringtun for WireGuard** (userspace, pure Rust, unprivileged-capable). A
   kernel-WireGuard netlink adapter keeps a slot behind the `ts-wg` trait.
5. **Ports-and-adapters.** `ts-engine` owns domain logic and speaks only to
   traits — `ControlClient`, `PacketConduit`, `TunDevice`, `Clock` — so every
   layer is testable without network or root.
6. **tokio runtime; Linux first.** TUN mode needs `CAP_NET_ADMIN`; userspace
   (`ts-net`) mode runs unprivileged. Windows second, macOS third.
7. **Typed state machines.** One `enum` per connection lifecycle so illegal
   states are unrepresentable.
8. **Wire parsers are panic-free on arbitrary input from day one**, and get a
   `cargo-fuzz` target as soon as they exist.
9. **Interop is the real acceptance test.** Official `tailscaled` + Headscale
   run in the test harness; one Go node stays in the test tailnet. Golden
   tests decode/encode captured frames byte-exact where deterministic.

### Non-goals (now)

Taildrop, Tailscale SSH, Funnel/Serve, exit-node hosting, mobile, and
reimplementing the control server. No FFI surface until two real non-Rust call
sites exist.

## Workspace layout

One crate per subsystem under `crates/` (`ts-types`, `ts-key`, `ts-control`,
`ts-derp`, `ts-stun`, `ts-disco`, `ts-magicsock`, `ts-wg`, `ts-tun`,
`ts-filter`, `ts-engine`, `ts-localapi`, `ts-daemon`, `ts-cli`, `ts-net`) plus
`xtask/` for the integration harness. Crates not yet reached by the phase plan
are placeholders with doc comments only.

## Phase plan

| Phase | Deliverable | Exit criterion |
|------:|-------------|----------------|
| 0 | Protocol recon | Annotated notes + corpus of captured frames (golden fixtures) |
| 1 | LocalAPI client + CLI | `ts-cli status/up/down/ping` work against real `tailscaled` |
| 2 | Control client (ts2021) | `ts-daemon --register` appears in Headscale, prints live netmap |
| 3 | DERP-only data plane | `ping 100.x.y.z` between two nodes via relay |
| 4 | TUN, routes, MagicDNS | Real apps (ssh, curl) work across the tailnet, relayed |
| 5 | Direct paths (disco/magicsock) | Two nodes behind distinct NATs upgrade to direct |
| 6 | Daemon surface + ACL filter | ts-daemon is a drop-in for daily use |
| 7 | Embeddable node + hardening | Example app serves HTTP on the tailnet from plain `cargo run` |

One milestone per PR; every milestone ships something runnable and is verified
end-to-end against the real stack, not just unit tests.

## Dependency policy

Small, auditable core; every dependency is justified here before entering the
tree.

| Crate | Used by | Justification |
|-------|---------|---------------|
| `serde`, `serde_json` | ts-types, ts-cli | LocalAPI and control protocol payloads are JSON; hand-rolling JSON is not a good use of risk budget. |
| `tokio` | ts-cli (net) | Async runtime decision fixed at project start; hyper requires it. |
| `hyper` + `hyper-util` + `http-body-util` | ts-cli | LocalAPI is HTTP/1.1 over a Unix socket. Hand-rolled HTTP parsing is error-prone (chunked encoding, header edge cases); hyper is the audited standard. `hyper-util` only for the `TokioIo` adapter; no connection pool — one conn per CLI invocation. |
| `thiserror` | all | Error ergonomics, zero runtime cost. |
| `tracing` | daemon/engine later | Structured logging; standard. |

Approved for later phases (not yet in tree): `boringtun`, `snow` (verify it can
express Tailscale's exact IK pattern + framing in Phase 0, else hand-roll over
`x25519-dalek` + `chacha20poly1305`), `x25519-dalek`, `chacha20poly1305`,
`blake2`/`sha2`, `rustls`, `tun`, `smoltcp` (ts-net only).

**Deliberately not used:** `clap` (the CLI has four subcommands and two flags;
~60 lines of hand-rolled argv parsing beats a large dependency for now — may be
revisited if the CLI surface grows), `hex` (two 30-line functions in ts-types),
`chrono`/`time` (see "timestamps" below).

## Phase 1 decisions (LocalAPI client + CLI)

- **Ground truth**: Go `ipnstate.Status` / `PeerStatus` / `PingResult` structs
  (`ipn/ipnstate/ipnstate.go`) and the LocalAPI client
  (`client/tailscale/localclient.go`) at tailscale v1.86; golden fixtures
  captured from a live tailscaled 1.86.2 on a real Headscale tailnet (see
  `crates/ts-types/tests/fixtures/`).
- **HTTP details mirrored from the Go client**: requests go to
  `http://local-tailscaled.sock/localapi/v0/...` over the UDS at
  `/var/run/tailscale/tailscaled.sock`; the `Host` header is
  `local-tailscaled.sock`. On Linux, authentication is by socket peer
  credentials — no token.
- **`up`/`down` are prefs edits** (`PATCH /localapi/v0/prefs` with a
  `MaskedPrefs` setting `WantRunning`), exactly what the Go CLI does for an
  already-authenticated node. Interactive login flows (`login-interactive`,
  IPN bus watch) are out of Phase-1 scope; initial auth is done with the
  official CLI + a Headscale preauth key. `up --authkey` support arrives with
  Phase 2, where we own registration anyway.
- **`ping` is `POST /localapi/v0/ping?ip=…&type=disco`** with the timeout knob
  left at the server default.
- **Timestamps stay as validated strings** (`Rfc3339` newtype wrapping the raw
  string) in Phase 1: the status CLI never does time arithmetic, Go emits
  RFC3339 with variable sub-second precision, and round-tripping byte-exact
  matters more for golden tests than a parsed representation. Revisit when the
  netmap layer (Phase 2) needs expiry math; expected resolution: hand-rolled
  RFC3339 parse or the `jiff` crate, justified then.
- **Key types are typed newtypes over `[u8; 32]`** (`NodePublic`,
  `MachinePublic`, `DiscoPublic`) that serde to/from Go's
  `"nodekey:<64 hex>"` / `"mkey:…"` / `"discokey:…"` encodings. Hex is
  hand-rolled (panic-free, fuzzable later).
- **`UserID` deserializes from both JSON number and string**: Go marshals it as
  a number in struct fields but as a string when it is a map key
  (`Status.User`).
- **Unknown fields are ignored, absent fields default.** The Go structs gain
  fields across versions; `deny_unknown_fields` would make the client brittle
  against the very binaries we interop with. Golden tests pin the fields we
  *do* read.
- **No connection reuse in the CLI**: one request per invocation, HTTP/1.1,
  `Connection: close` semantics. Simplest correct thing.

## Interop environment (manual until xtask lands)

- Headscale 0.26 runs in Docker (`headscale/headscale:0.26`), config in
  `interop/headscale/`; SQLite state; listens on `127.0.0.1:8080`.
- Official tailscaled 1.86.2 static binaries run with
  `--tun=userspace-networking` (no root TUN needed), one state dir + socket
  per node, joined via preauth keys to the local Headscale.
- This gives a real tailnet (2+ nodes) whose LocalAPI socket Phase 1 is
  verified against, and whose traffic Phase 0 captures.
- Headscale refuses to start with an empty DERP map, so its embedded DERP
  server is enabled (region 999, STUN on :3478). Observed: tailscaled 1.86
  cannot connect to it over the plain-HTTP `server_url` (health warning
  "could not connect to the 'Headscale Embedded DERP' relay server") — the
  DERP client wants TLS. Harmless for Phases 1–2: the two same-host nodes
  establish *direct* paths from netmap endpoints alone (verified:
  `tailscale ping` reports `via 192.0.2.2:41642`, ~1–2 ms). Phase 3
  (DERP-only data plane) must fix this — likely TLS in front of Headscale or
  a standalone `derper`-equivalent test relay.

## Deviations from Go behavior

(Record every deviation here as it is discovered/decided.)

- `ts-cli status` renders a subset of `tailscale status` columns (IP, hostname,
  user, OS, connection state); it does not yet implement the Go client's
  peer-sorting-by-DNS-name tiebreak rules beyond simple name sort. Golden
  fixtures pin JSON decoding, not human-readable rendering.
