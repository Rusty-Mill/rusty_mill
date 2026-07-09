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
| `tracing` | daemon/engine | Structured logging; standard. |
| `x25519-dalek`, `chacha20poly1305`, `blake2`, `hkdf` | ts-control | Noise IK primitives (hand-rolled controlbase); see Phase 2 decisions. |
| `h2`, `http`, `bytes` | ts-control | HTTP/2 over the Noise channel, mirroring Go's `x/net/http2`. |
| `crypto_box` | ts-derp | NaCl box (X25519 + XSalsa20-Poly1305) for the DERP ClientInfo/ServerInfo handshake — exactly the construction Go uses. |
| `boringtun` | ts-wg | Userspace WireGuard, pure Rust, unprivileged; the strategic WG choice. |
| `libc` | ts-tun | TUN device creation/config via raw ioctls; syscall bindings only, no iproute2. |
| `crypto_box` | ts-disco | NaCl box for disco messages (same primitive as DERP; no new dep). |
| `rand_core` | ts-stun, ts-magicsock | STUN transaction ids and disco ping tx ids. |

Approved for later phases (not yet in tree): `snow` (evaluated in Phase 2 and
rejected — see Phase 2 decisions), `sha2`, `rustls` (HTTPS DERP + hosted
control plane), `smoltcp` (ts-net only, Phase 7). The `tun` crate was
considered for Phase 4 and passed over in favor of hand-rolled ioctls (see
Phase 4 decisions); it may return for the Windows `wintun` adapter.

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

## Phase 2 decisions (control client)

Full protocol notes are in `PROTOCOL.md` (the Phase-0 recon artifact); the
*decisions* live here.

- **Hand-rolled Noise IK, not `snow`.** The plan said to verify snow could
  express Tailscale's exact IK pattern and framing in Phase 0, else
  hand-roll. Verdict: hand-roll. The handshake is standard IK, but the
  transport uses Tailscale's own record framing (`[type][BE len][ct]`, 4 KiB
  cap) and **big-endian** record nonces (Noise mandates little-endian), so
  snow's transport is unusable and its handshake value shrinks below the
  framing we'd still own. Built on `x25519-dalek` + `chacha20poly1305` +
  `blake2`/`hkdf`. Verified **byte-exact** against the real Go `controlbase`
  package via a tiny Go echo server (`interop/noise-server-go/`,
  `crates/ts-control/tests/go_interop.rs`) — the strongest interop signal
  short of Headscale itself.
- **HTTP/2 over Noise via the `h2` crate**, mirroring Go's `x/net/http2` on
  the noise conn. Prior-knowledge h2c directly on the secured stream: no
  TLS, no ALPN, no h2c upgrade — the server speaks h2 immediately. Our
  `controlbase::Conn` implements tokio `AsyncRead`/`AsyncWrite` so `h2` sits
  straight on top.
- **Compression off** (`MapRequest.Compress = ""`). We skip zstd for now:
  the map stream stays plain length-prefixed JSON, which keeps the frame
  reader trivial and fuzzable. Revisit if a real tailnet's netmap size makes
  it worth a `zstd` dependency.
- **Map stream framing**: 4-byte **little-endian** length + JSON per frame
  (note the endianness flip vs. the big-endian Noise records — both are
  mirrored from Go and easy to get wrong). Keep-alive frames
  (`{"KeepAlive":true}`) are surfaced but carry no map data.
- **Early payload handled but unused.** We read the optional
  `\xff\xff\xffTS`-prefixed early payload (node-key challenge) off the
  plaintext stream before starting h2, and discard it — preauth-key
  registration doesn't need the challenge. Headscale sends none; the code
  path exists so a hosted-control-plane connection won't desync.
- **Our own state file**, `ts-rs.state.json` (0600), holding the three
  private keys as `privkey:<hex>`. Go-state-file compatibility is a
  non-goal; identities are per-daemon.
- **Registration is preauth-key only.** Interactive login (`AuthURL`
  visit / followup polling) is surfaced as a typed error but not driven;
  it belongs with the LocalAPI/IPN-bus work in Phase 6.
- **Deferred to the hosted-control-plane milestone**: HTTPS (:443) dialing,
  the 80/443 race, DNS bootstrap, and OS-root-store TLS. Phase 2 targets
  Headscale over plain HTTP only.

## Phase 3 decisions (DERP-only data plane)

First real connectivity. Full protocol notes in `PROTOCOL.md`; decisions here.

- **DERP over plain HTTP against Headscale's embedded relay.** Headscale's
  embedded DERP (region 999) accepts the `GET /derp` + `Upgrade: DERP`
  handshake over plain HTTP on its `server_url` port, and the greeting frame
  carries the server's node key — so Phase 3 needs **no separate TLS relay**
  and no `rustls` yet. (This is also why the Phase-2 note about tailscaled
  refusing plain-HTTP DERP doesn't block us: *our* client has no such
  restriction.) `rustls`/HTTPS DERP arrives with the hosted-control-plane and
  direct-path milestones.
- **NaCl box via `crypto_box`.** DERP's ClientInfo/ServerInfo frames use
  `crypto_box` (X25519 + XSalsa20-Poly1305, 24-byte nonce prepended). Added
  the RustCrypto `crypto_box` crate rather than hand-roll XSalsa20-Poly1305 —
  it's exactly this construction and only the handshake needs it. Relayed
  Send/Recv frames are opaque (WireGuard already encrypted them), so the hot
  path is pure framing.
- **boringtun for WireGuard, behind the `WgPeer` trait.** `ts-engine` depends
  only on `ts_wg::WgPeer`; `BoringWgPeer` wraps boringtun's per-peer `Tunn`.
  The node key is the WG static, the peer node key is the peer static — the
  same key DERP routes by, so no key mapping is needed. The kernel-WG netlink
  adapter will implement the same seam later.
- **The "magic socket" is trivial in Phase 3.** WireGuard is oblivious to the
  path: the engine takes each `WgAction::ToPeer(datagram)` and ships it in a
  DERP SendPacket frame keyed by the peer node key; inbound RecvPacket frames
  feed straight into `decapsulate`. Path discovery and direct-path migration
  are Phase 5 — this is deliberately the dumbest possible transport so WG and
  session bugs surface in isolation.
- **Userspace ICMP instead of a TUN device.** Phase 4 owns TUN; to prove
  relayed connectivity now, the engine answers ICMP echo requests to its own
  tailnet IP and originates echo requests for `EngineHandle::ping`, all in
  userspace (`ts-engine::icmp`, panic-free, checksum-verified). No root, no
  TUN. Verified: two `ts-daemon` nodes on a Headscale tailnet ping each
  other's `100.64.x.y` over the relay (`pong … via DERP relay in ~6 ms`).
- **DERP URL derived from the control URL.** For Phase 3 the daemon defaults
  `--derp-server` to `--login-server` (Headscale's embedded DERP shares the
  host). Real DERP-map region selection from the netmap `DERPMap` is a
  Phase-5 concern; the `--derp-server` flag is the escape hatch until then.
- **No DERP reconnection yet.** If the relay connection drops the engine logs
  and stops; automatic reconnection with backoff is a robustness item for the
  daemon-hardening phase (Phase 6).

## Phase 4 decisions (TUN, routes, MagicDNS)

Real apps across the tailnet. Full protocol notes in `PROTOCOL.md`.

- **Hand-rolled TUN via ioctls, not the `tun` crate.** The `tun` crate pulls
  Windows bindings and a wider surface; TUN on Linux is a small, stable set
  of ioctls (`TUNSETIFF` + `SIOCSIF*`). `ts-tun` does them directly over
  `libc` — pure syscalls, in the sovereign-stack spirit, one dependency
  (`libc`). The `tun` crate remains the likely choice for the Windows
  (`wintun`) adapter in Phase 7.
- **No route command: the `/10` netmask trick.** Assigning the TUN address
  as `100.64.x.y/10` makes the kernel auto-install the connected route for
  the whole CGNAT range, so we never shell out to `ip route` or hand-roll
  netlink route management. Verified on a real device.
- **Userspace L4 checksum fixup.** Kept the Phase-3 userspace ICMP path for
  no-TUN mode, but TUN mode hands decrypted packets to the OS. Locally
  generated TCP/UDP can arrive at the TUN with `CHECKSUM_PARTIAL`; the engine
  recomputes the transport checksum (`ts-engine::l4`) before relaying.
  Idempotent for already-correct checksums, so it's a safe always-on
  correctness net. (This environment's TUN happened to deliver complete
  checksums, but real offloading NICs/stacks need it.)
- **Proactive WireGuard handshake on peer discovery**, so the first
  connection succeeds instead of dropping its SYN during the handshake —
  what tailscaled does. Made `apply_netmap` async to relay the handshake
  datagrams.
- **MagicDNS = hosts-file stub** (as the plan specifies "hosts-style stub
  first"): a managed, marker-delimited block written to a configurable hosts
  file (`--hosts-file`), rebuilt from the netmap. A real resolver on
  100.100.100.100:53 is deferred to daemon hardening (Phase 6).
- **Engine data path stays behind the ports**: the TUN is `Option<Tun>` in
  the engine; absent → Phase-3 userspace ICMP (unprivileged), present →
  Phase-4 OS datapath (needs `CAP_NET_ADMIN`). The `ts_wg::WgPeer` and DERP
  seams are unchanged — only local delivery/origination swapped.

### Phase 4 verification harness

`interop/tun_netns_test.sh`: a bridge (`br-ts`, 10.0.0.1) with two network
namespaces (`ns-a`, `ns-b`), each a veth to the bridge and a `ts-daemon` with
a real TUN, all joined to the host's Headscale (`server_url`
`http://10.0.0.1:8080`) and its embedded DERP. Runs `python -m http.server`
bound to ns-b's tailnet IP and `curl`s it from ns-a. **Verified**: ICMP ping
(3/3, ~1.4 ms) and TCP `curl` both succeed across the tailnet, relayed via
DERP, through real TUN devices — proving real apps work. (This is the
embryo of the `xtask` netns harness the plan calls for.)

## Phase 5 decisions (direct paths — the hard one)

Endpoint discovery, disco, and live DERP→direct migration. Protocol in
`PROTOCOL.md`.

- **STUN and disco reuse `crypto_box`** (already in-tree for DERP); no new
  crypto dependency. `ts-stun` sends the minimal RFC-5389 binding request
  (no SOFTWARE/FINGERPRINT), which real servers answer.
- **magicsock is single-task-owned, no locks.** It lives inside the engine's
  event loop (`&mut self`); the UDP socket is shared via `Arc` so the loop
  can `recv_from` in one `select!` arm while magicsock sends from others.
  Typed per-peer path state (`Relay` ↔ `Direct(addr)`) so an unverified path
  can't be used.
- **The disco-key gotcha (empirical).** Headscale only propagates a node's
  disco key to peers after the node reports endpoints — and it *ignores*
  endpoints/disco on the streaming map request. They must be sent via a
  separate **lite** map request (`Stream=false`, `OmitPeers=true`);
  `ts-control::update_endpoints` does this at startup. Without it, peers see
  a zero disco key and the disco box fails to open, so NAT traversal never
  begins. This cost real debugging time (the plan warned Phase 5 would).
- **The Docker FORWARD-DROP gotcha (empirical).** Docker sets the host
  iptables `FORWARD` policy to `DROP`, and `br_netfilter` routes bridged
  traffic through it — silently dropping the direct node↔node UDP path (all
  earlier phases' traffic went *through* the host, so this only surfaced
  now). The harness adds `iptables -I FORWARD -i br-ts -o br-ts -j ACCEPT`.
- **Path liveness**: a verified direct path with no pong for 15 s falls back
  to DERP; heartbeat re-pings every 5 s keep NAT mappings open. call-me-maybe
  is re-sent every 2 s until a direct path exists (the first can race the
  peer's DERP registration).
- **Scope of verification.** On a flat L2 (both nodes' local endpoints
  mutually reachable) two nodes reliably upgrade DERP→direct (3/3 runs) and
  carry real TCP (`curl`) over the direct path with the WireGuard session
  intact — the complete mechanism: discovery → disco handshake → upgrade →
  live migration. **Deferred**: a full two-distinct-NAT harness (netns
  routers + masquerade + STUN server) and symmetric-NAT hole punching — the
  empirically hardest case (Linux `MASQUERADE` is symmetric; cone NAT needs
  special setup). The code path for it exists (`--stun`, reflexive discovery,
  `interop/stun_server.py`); wiring the NAT harness is the next increment.

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
