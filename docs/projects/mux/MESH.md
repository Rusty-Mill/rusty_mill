# Own mesh network (proposed C80): design note

Status: Proposed (2026-11-13). Nano answered the six questions below on 2026-11-13 17:12Z (all
recommendations concurred, with two comments, recorded under "Answers"); each build step still
needs its own go (D37). Docs only. Nothing here is built, and starting any stage needs
Nano's go (D37). Answers Nano's question: "Instead of tailnet, can we develop our own
meshnet, tailnet-like, and have it be part of the mux server?" Decision rows D242-D253 are
**Proposed** until Nano accepts them on the PR.

## Short answer

**Yes, and most of it is already in the repo.** `crates/apps/rusty_tailscale` is a pure-Rust
tailnet client: WireGuard data plane (`ts-wg`, boringtun), DERP relay client (`ts-derp`),
STUN and disco hole punching (`ts-stun`, `ts-disco`, `ts-magicsock`), and `ts-net`, an
embeddable node with a userspace TCP/IP stack (smoltcp) that needs **no TUN device, no root
and no `tailscaled`**. Every dependency it uses is already in the workspace.

What it does not have is the other half of a tailnet: the **coordination server** (who is in
the mesh, their keys and addresses) and the **relay server** (DERP and STUN). Those come from
Headscale or Tailscale today, and `rusty_tailscale/DESIGN.md` lists "reimplementing the
control server" as a non-goal. So "our own mesh" is two moves:

1. **Embed** a `ts-net` node in `rusty-mux-server`. This replaces `tailscaled` and its
   LocalAPI (#1665's dependency) and leaves Tailscale or Headscale as the coordinator.
2. **Replace the coordinator** with a small first-party hub (coordination + DERP + STUN) that
   speaks the subset of Tailscale's protocol the embedded node already speaks.

Each move stands alone. Move 1 is days of work; move 2 is weeks and is the real "own it".

## Shape: one server, many clients

Like Tailscale, the mesh has a **server side** and **clients**. The server side is the
**hub**, shipped with the mux server (`rusty-mux-server --hub`, D245). It has two jobs that
scale differently:

- **Coordinator:** who is in the mesh, their keys and addresses. One active per mesh.
- **Relays** (DERP and STUN): carry traffic when two nodes cannot connect directly. Many per
  mesh; the coordinator hands nodes a list and they pick the nearest. Any mux server with a
  public address can add itself as a relay.

Every other mux server and client is a plain node with the embedded `ts-net`. Every mux
server can run the hub code, but only one coordinator is switched on per mesh. Separate
meshes (one hub each) are fine; joining meshes (federation) is out of scope.

## Goals and non-goals

Goals:

- A mux server reachable from any of the owner's machines through NAT, with no port forward,
  no VPN product to install, and no root.
- Peer identity that comes from keys we control, feeding the one access table (shared mode,
  #1662; `ts:LOGIN` and `ts-node:NAME` principals, #1665).
- Relay first, direct paths as an upgrade (the order `rusty_tailscale` already follows).
- Small: no new third-party dependency, no new wire value in the mux protocol.

Non-goals:

- A general-purpose VPN: no TUN, no subnet routes, no exit nodes, no MagicDNS, no Taildrop.
  The mesh carries mux connections only.
- Compatibility with every Tailscale client feature or the hosted control plane.
- Replacing the SSH bootstrap (D 2026-10-29) or the TLS listener (#506). They stay as the
  zero-setup and the no-mesh paths.
- Windows and macOS in the first stages (see Risks).

## Components

| Component | Job | Status in tree |
|---|---|---|
| Peer identity | Node keypair (X25519), disco keypair; machine key for the control channel | `ts-key` |
| Control client | Noise IK channel (ts2021), register, netmap long-poll, endpoint updates | `ts-control` |
| Data plane | WireGuard per peer behind the `WgPeer` trait | `ts-wg` over boringtun |
| Path mux | One virtual transport per peer: DERP first, direct once disco proves it | `ts-magicsock`, `ts-disco`, `ts-stun` |
| ACL filter | Netmap packet filter for inbound packets | `ts-filter` |
| Userspace stack | `Node::bind(port)` yields `rusty_tokio` streams | `ts-net` |
| **Coordination server** | Register nodes by pre-auth key, hold the node table, push netmaps, propagate disco keys and endpoints | **missing** |
| **Relay server** | DERP server (frame codec exists in `ts-derp`), admits only registered node keys | **missing** |
| **STUN server** | Reflexive-address replies (client exists; only a Python test server) | **missing** |
| Admission | Peer address to node to principal, then the shared-mode allowlist | `tailnet::Whois` asks `tailscaled`; becomes an in-process netmap lookup |

The three missing pieces form one **hub**. A hub needs a publicly reachable address, so it
runs on one machine with an open port (a small VPS, or any mux server with a forwarded port).
Mux servers behind NAT are plain nodes.

## What it replaces

| Today (#1657, #1665) | With the mesh |
|---|---|
| `tailscaled` must run on the host, with root for its TUN | The server embeds its own node; no daemon, no root |
| Peer identity: HTTP call to `tailscaled`'s LocalAPI `whois` (5 s, 16 at once) | In-process lookup of the peer's tailnet IP in the node's netmap; same `ts:LOGIN` and `ts-node:NAME` principals |
| Bind to a `100.64.0.0/10` address on a host interface | `Node::bind(port)` on the userspace stack; the address rule is moot, the interface does not exist on the host |
| Coordinator: Tailscale SaaS or Headscale | Stage 1: unchanged. Stage 2: our hub |
| Allowlist, guest rules (D232, D238, D239), no descriptors, no UDP | **Unchanged.** One access table, one `conn::run` over generic halves |

This changes C27's decided shape (D235: ask `tailscaled` whois). The tailnet listener stays as the option for a host that already runs `tailscaled`. Both feed
the same allowlist, so neither is wasted.

Mux's own UDP transport (C13) is not used inside the mesh: WireGuard already roams on the
peer's next authenticated datagram, so running mux's roaming layer on top would be a second
copy of the same mechanism. Mesh peers use TCP streams; C13 stays for the SSH path.

## Staged path

1. **Stage 1: embedded node (identity and relay from the existing coordinator).** Feature
   `mesh` in `rusty_mux_server`: start a `ts-net` node, `bind` the mux port, run `conn::run`
   on each accepted stream, derive the principal from the netmap. Joins a Headscale or
   Tailscale tailnet through a pre-auth key. DERP is whatever the coordinator advertises.
2. **Stage 2: first-party hub.** Coordination (ts2021 server side, register, map long-poll,
   pre-auth keys, disco-key and endpoint propagation), DERP server, STUN server. The embedded
   node is unchanged; only the URL it registers against moves. Admission by pre-auth key
   tied to a login name, so `ts:LOGIN` principals come from our own table.
3. **Stage 3: direct paths.** The code exists (`ts-magicsock`, Phase 5). What is left is
   evidence: the two-NAT harness and the symmetric-NAT case, which DESIGN.md defers. The mesh
   works without it; direct paths only take load and latency off the relay.

4. **Stage 4: coordinator failover.** The coordinator's state (node table, keys, pre-auth
   keys) is small. The active coordinator streams it to one or more standby mux servers; nodes
   hold a list of coordinator addresses from registration. If the active one dies, an operator
   promotes a standby (`--hub --promote`). Manual promotion on purpose: it cannot split-brain
   (two coordinators handing out conflicting data). Automatic election (Raft-style) is out of
   scope: new consensus code, hard to test, a worse failure when wrong. Until stage 4, and
   even after, a dead coordinator is a soft outage: existing WireGuard sessions keep working;
   only new joins, key changes and revocations wait.

"Identity and relay first, direct paths later" is therefore stages 1-2 then 3, in the same
order `rusty_tailscale` was built (DERP-only data plane before NAT traversal).

## Protocol choice: compatible subset, not a new protocol

Option A (recommended): the hub speaks the ts2021 subset the node already speaks. Pros: the
client crates are built and golden-tested; official `tailscaled` is a free interop test peer;
a phone or laptop can join with the stock client. Cons: we track a protocol Tailscale owns
(pinned to v1.86.2), and the server side of ts2021 (Noise responder, HTTP/2 over Noise, netmap
JSON) is the largest single piece of work.

Option B: our own small protocol (Noise IK over UDP, an address table, a relay). Pros:
smaller and ours. Cons: loses the client crates and the interop oracle, and every byte of it
is new security-critical code. Not recommended while Option A's client side is already done.

## Crypto choice

No hand-rolled crypto. Primitives and the WireGuard state machine are not written here.

| Option | New dependency? | Notes |
|---|---|---|
| **boringtun 0.7 (in use)** | No: already `[workspace.dependencies]` and used by `ts-wg` | Userspace WireGuard, pure Rust, Cloudflare. Sits behind the `WgPeer` trait, so it can be swapped. **Recommended for now.** |
| Control channel: `x25519-dalek`, `chacha20poly1305`, `blake2`, `hkdf`, `crypto_box` | No: already in tree for `ts-control`, `ts-derp`, `ts-disco` | The ts2021 responder reuses them. |
| `snow` (Noise framework) | Yes | The `rusty_tailscale` team evaluated and rejected it in Phase 2. Needed only for Option B. |
| First-party Noise from `rusty_pk` (X25519), `rusty_aead` (ChaCha20-Poly1305) and `rusty_sha2` | No | Fits "own the stack" and UDP-TRANSPORT's cipher choice, but composing a handshake is new protocol crypto and needs an independent review. A later spike (`rusty_wg` behind `WgPeer`), not a stage. |
| Kernel WireGuard (netlink) | No | Needs root and a TUN-like interface; against the goal. |

Stage 1 adds **no third-party dependency**, but it does add a workspace edge from
`rusty_mux_server` to `ts-net`/`ts-engine`, which pulls boringtun and smoltcp into that
binary behind the `mesh` feature. **ADR-0003 forbids that edge** (apps do not depend on app
crates), and the closure is 13 of the 14 `rusty_tailscale` crates, so stage 1 first needs those
crates moved to `libs/` (the move gate of D233; Q54, Q60). Stage 2 adds none either; a new crate for the hub is an
owner gate (question 4).

## Spike plan

Three timeboxed spikes, each with a pass/fail test, none merged as product code.

| Spike | Builds | Pass | Fail |
|---|---|---|---|
| **A: embedded node** (1-2 days) | `mesh` feature: `ts-net` node in `rusty-mux-server`, joined to the Headscale in `rusty_tailscale/interop/headscale/` | With **no `tailscaled` running and no root**, a second node runs `rusty-mux` against the first: a 1 MiB paste round-trips byte-exact. An allowlisted `ts:` principal is admitted; an unlisted one is closed before `Hello` | Needs root or a TUN device; or identity cannot be read from the netmap without `tailscaled`; or one command through the mesh is lost or reordered |
| **B: hub** (1-2 weeks) | Minimal hub: ts2021 responder, register with pre-auth key, netmap with two nodes, DERP server, STUN server | Two **unmodified** embedded nodes register against the hub (not Headscale), see each other in the netmap, and move 4 MiB via our DERP with matching SHA-256. Then stock `tailscaled` 1.86.2 joins the hub and pings a node | Any node needs a patched client crate; or the hub relays for an unregistered key |
| **C: direct path** (3-5 days, after B) | The two-NAT netns harness with the STUN server | Cone NAT: the path moves from DERP to direct within 10 s and a transfer survives the move. Symmetric NAT: stays on DERP and the transfer still completes | Transfer stalls during the move; or symmetric NAT leaves the connection dead instead of relayed |

Spike A is also the cheapest answer to "is this worth doing": it removes `tailscaled` from
the story with almost no new code.

## Effort

| Stage | Size | Mostly |
|---|---|---|
| 0 move crates | 2-3 days | move 13 `rusty_tailscale` crates to `libs/` (ADR-0003, D233 gate), no behaviour change |
| 1 embedded node | 3-5 days | glue, a netmap lookup behind the `Whois` interface, feature gating, tests |
| 2 hub | 2-4 weeks | the ts2021 server side (~2-3k lines), DERP server (~1k), STUN (~200), pre-auth key store, tests against stock `tailscaled` |
| 3 direct paths | 1 week | the NAT harness and the symmetric-NAT evidence |
| 4 failover | 1 week | state replication to standbys, address list in the netmap, `--promote`, a kill-and-promote test |

Estimates are for an agent with review turnaround, not calendar time.

## Risks

- **Protocol tracking.** ts2021 and netmap fields are Tailscale's. Pin the version, golden
  tests, and treat unknown fields as ignorable. A break costs a hub update, not a redesign.
- **The hub is a new internet-facing service.** DERP is an open relay if it admits unknown
  keys: it must relay only for node keys in the table. Rate limits, a per-key bandwidth cap,
  and no logs of payload. Independent security review before stage 2 ships.
- **Coordinator as a single point.** If it is down, existing connections continue
  (WireGuard sessions stay up) and new ones fail. Stage 2 accepts that; stage 4 adds a
  manually promoted standby. Relays are not single points: a mesh can list several.
- **Platforms.** `ts-magicsock` uses `platform-linux`'s UDP socket and `ts-tun` is Linux-only,
  so the mesh is Linux first. Both ends of a mesh connection need a node, so a Linux-only mesh
  also excludes Nano's Windows client (D51, D227), not just the server. Windows is Nano's main
  platform (Answers), so the port of `ts-magicsock` to `WindowsUdpSocket` (spike A-W) follows
  spike A directly; until then Windows keeps SSH and (later) TLS.
- **Key rotation** is not implemented in `rusty_tailscale` (DESIGN.md, Phase 6). A leaked node
  key stays valid until the hub deletes the node; stage 2 must ship revoke.
- **Weight.** boringtun and smoltcp enter the server binary. Behind `mesh`, off by default.
- **Fully-owned WireGuard** is a larger job than the rest of this note combined and is not
  needed to meet the goal. It stays a later spike.

## Decisions (Proposed, D242-D253)

The `ts:` and `ts-node:` principals are #1665's (open, not yet in the pack). Rows for the pack (`docs/projects/rusty_term/handoff/DECISIONS.md`) are appended by
`rusty-term-expert`; IDs are reserved here so parallel PRs do not clash (D240-D241 are #1665's).

| ID | Proposal |
|---|---|
| D242 | Build the mesh as stages 1-3 above: embedded node, then first-party hub, then direct paths |
| D243 | The mesh data plane reuses `rusty_tailscale` (`ts-net`, boringtun); no new third-party dependency in any stage |
| D244 | The hub speaks the ts2021 subset the embedded node speaks (Option A); this lifts `rusty_tailscale`'s "no control server" non-goal for the hub only |
| D245 | The hub is a role of the mux codebase, not a new service: a library launched by `rusty-mux-server --hub` and by `ts-daemon hub`; the crate itself is a separate owner gate |
| D246 | Spikes A, B, C with the pass/fail tests above; B starts only after A passes |
| D247 | Mesh peers use TCP streams; mux UDP transport (C13) is not run inside the mesh |

## Answers (Nano, 2026-11-13)

Nano concurred with all six recommendations, with two comments.

- **Q2:** Tailscale-compatible hub, and **real Tailscale stays a supported option** for a user
  who already has a tailnet, like SSH next to the mux's own transport. So the tailnet listener
  (#1665) and the embedded node joining Tailscale or Headscale are kept, not retired.
- **Q6:** Windows is Nano's main platform, so **Linux-only is not a comfortable end state**.
  Windows is a priority, not a footnote. A cheaper path than the note first assumed:
  `rustils` already has `platform-windows::WindowsUdpSocket`, and `ts-magicsock` is gated to
  `target_os = "linux"` only because it names `LinuxUdpSocket`. Spike A runs on Linux (the
  harness is there); a spike A-W then ports `ts-magicsock` to the `platform::net::UdpSocket`
  port so the same node runs on Windows (pass: spike A's test, run client and server on
  Windows, no `tailscaled`, no admin). The embedded node needs no TUN, so `wintun` is not
  needed for the mesh.

## Questions for Nano (answered above)

1. **Go on spike A (embedded node, 1-2 days)?** Recommend yes: it removes `tailscaled`, adds
   no dependency, and tells us how much of the mesh we already own.
2. **Hub protocol: Tailscale-compatible subset (Option A) or our own (Option B)?** Recommend A:
   the client side is built, and stock `tailscaled` becomes the interop test.
3. **Lift the "no control server" non-goal in `rusty_tailscale` for the hub?** Recommend yes,
   but only once spike A passes, and only for a hub that serves our nodes, not a Headscale
   clone.
4. **Hub home: a new `ts-hub` crate (an owner gate), or a module inside `ts-control`?**
   Recommend a new crate `ts-hub` under `rusty_tailscale`: the server side shares no code
   path with the client, and `rusty-mux-server --hub` stays a thin launcher.
5. **WireGuard: keep boringtun, or start an in-house `rusty_wg` spike?** Recommend keep
   boringtun now and revisit after stage 2; it sits behind `WgPeer`, so the swap is local.
6. **Platforms: is a Linux-only mesh acceptable for stages 1-2, knowing it excludes the
   Windows client?** Recommend yes. Windows keeps SSH and the TLS listener; a Windows UDP adapter is its own card. *(Answered: Windows is a priority; see Answers.)*
