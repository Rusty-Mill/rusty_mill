# ts2021 control protocol notes (Phase 0 recon)

Ground truth: `tailscale.com` Go module **v1.86.2** (fetched from
proxy.golang.org; GitHub is out of scope for this environment). File
references below are into that tree. Capability version at this release:
**123** (`tailcfg/tailcfg.go`).

This document covers what Phase 2 (control client) needs. DERP and disco
sections will be added in their phases.

## Layer cake

```
TCP :80/:443 (Headscale: :8080, plain HTTP)
 └─ HTTP/1.1 POST /ts2021 + Upgrade  (control/controlhttp/client.go)
     └─ Noise IK secure channel      (control/controlbase/{handshake,conn}.go)
         └─ [optional early payload] (internal/noiseconn/conn.go)
         └─ HTTP/2 client conn       (internal/noiseconn/conn.go)
             ├─ POST /machine/register   JSON RegisterRequest/Response
             └─ POST /machine/map        JSON MapRequest → framed MapResponse stream
```

## Key encodings (`types/key`)

X25519 keys, prefixed lowercase hex over JSON:
- machine public: `mkey:<64 hex>`; node public `nodekey:…`; disco public
  `discokey:…`; private keys `privkey:…` (state files only, never on the
  wire).

## Fetching the control server's Noise key

`GET <server>/key?v=<capver>` (plain HTTPS/HTTP, *not* Noise) returns
`tailcfg.OverTLSPublicKeyResponse`:

```json
{"legacyPublicKey": "mkey:…", "publicKey": "mkey:…"}
```

`publicKey` is the Noise static (`controlKey` below). Headscale serves this
at the same path.

## controlbase: Noise IK handshake (`control/controlbase/handshake.go`)

Instantiation: `Noise_IK_25519_ChaChaPoly_BLAKE2s`.

- `h = BLAKE2s-256(protocolName)` (name is 34 bytes, so hashed);  `ck = h`.
- Prologue mixed first: ASCII `"Tailscale Control Protocol v"` + decimal
  protocol version (= capability version, e.g. `"…v123"`), via
  `MixHash`.
- Then `MixHash(controlKey)` (`<- s` pre-message).
- `MixDH(priv, pub)`: `HKDF-BLAKE2s(salt=ck, ikm=X25519(priv,pub), info=∅)`
  → first 32 bytes replace `ck`, next 32 bytes are the message key
  (RFC 5869; identical to Noise HKDF when info is empty).
- Handshake AEAD calls: ChaCha20-Poly1305 with **all-zero nonce** (each key
  used exactly once), AAD = current `h`; ciphertext mixed via `MixHash`.
- `Split()`: `HKDF-BLAKE2s(salt=ck, ikm=∅, info=∅)` → k1 (client→server),
  k2 (server→client).

### Message layouts (`control/controlbase/messages.go`)

Initiation, client→server, **101 bytes** total:

| bytes | content |
|-------|---------|
| 2 | protocol version, big-endian (=123) |
| 1 | message type `0x01` |
| 2 | payload length, BE (=96) |
| 32 | client ephemeral public (cleartext) `e` |
| 48 | client machine public, encrypted `s` (32+16 tag) |
| 16 | tag of empty payload (`ss`) |

Response, server→client, **51 bytes** total:

| bytes | content |
|-------|---------|
| 1 | message type `0x02` |
| 2 | payload length, BE (=48) |
| 32 | control ephemeral public (cleartext) `e` |
| 16 | tag of empty payload (`ee, se`) |

Error frame (server→client, instead of response): type `0x03`, 2-byte BE
length, then that many bytes of unauthenticated UTF-8 error text — surface
as a hint, do not trust.

## controlbase: transport (`control/controlbase/conn.go`)

Record frames: `[1B type=0x04][2B BE ciphertext-len][ciphertext]`.

- Max frame size **4096** bytes total ⇒ max plaintext 4096−3−16 = 4077.
- AEAD: ChaCha20-Poly1305, AAD **empty**, nonce = 4 zero bytes + **8-byte
  big-endian counter** starting at 0, incremented per record, per
  direction. ⚠️ Big-endian deviates from the Noise spec (little-endian);
  this is Tailscale-specific and is why an off-the-shelf Noise library's
  transport phase cannot be used as-is (see DESIGN.md).
- Zero-length records are legal (used as h2 keep-alive carrier).
- A failed decrypt permanently poisons the connection.
- Nonce exhaustion (2⁶⁴−1) closes the connection; in practice the client
  re-dials long before.

## controlhttp: the upgrade dance (`control/controlhttp/client.go`)

1. Dial TCP (Headscale: the `server_url` host:port, plain HTTP).
2. Send `POST /ts2021` with headers:
   - `Upgrade: tailscale-control-protocol`
   - `Connection: upgrade`
   - `X-Tailscale-Handshake: <base64(std) of the 101-byte initiation>`
     (RTT optimization; the server accepts the initiation from the header)
3. Expect `101 Switching Protocols` with the same `Upgrade` value echoed.
4. The raw TCP stream then carries the Noise response (51 bytes) followed
   by Noise transport records. (If the header wasn't sent, the initiation
   would be written post-upgrade; we always send the header like Go does.)

Production also supports TLS :443 and an 80/443 race with fallback; not
needed for Headscale-over-HTTP and deferred.

## Early payload (`internal/noiseconn/conn.go`)

After the handshake, *inside* the Noise plaintext stream, the server MAY
send: magic `FF FF FF 54 53` (`"\xff\xff\xffTS"`), then 4-byte **BE**
length, then JSON `tailcfg.EarlyNoise` (`{"version":…,
"nodeKeyChallenge":"chalpub:…"}`). If the first 9 plaintext bytes don't
start with the magic, they are the beginning of the HTTP/2 stream and must
be preserved. Headscale 0.26 does not send an early payload.

## HTTP/2 over Noise

The client runs an HTTP/2 **client connection directly over the Noise
stream** (Go: `http2.Transport.NewClientConn`; ours: `h2::client`).
Requests use normal `https://<host>/...` URLs but the connection is the
Noise channel — no TLS, no ALPN; the server routes by `:path`.

### `POST /machine/register` — `tailcfg.RegisterRequest` (JSON)

Fields we send (subset; `tailcfg/tailcfg.go`):

```json
{
  "Version": 123,
  "NodeKey": "nodekey:…",
  "OldNodeKey": "nodekey:…(zero if none)",
  "Auth": {"AuthKey": "<headscale preauth key>"},
  "Expiry": "0001-01-01T00:00:00Z",
  "Followup": "",
  "Hostinfo": {"IPNVersion": "…", "Hostname": "…", "OS": "linux"}
}
```

Response `tailcfg.RegisterResponse`: `User`, `Login`, `NodeKeyExpired`,
`MachineAuthorized`, `AuthURL` (set ⇒ interactive auth pending), `Error`.

### `POST /machine/map` — `tailcfg.MapRequest` (JSON)

```json
{
  "Version": 123,
  "Compress": "",            // we never ask for zstd (see DESIGN.md)
  "KeepAlive": true,
  "NodeKey": "nodekey:…",
  "DiscoKey": "discokey:…",
  "Stream": true,            // long-poll: server pushes updates
  "Hostinfo": {…},
  "Endpoints": [],
  "OmitPeers": false
}
```

Response body is a **stream of frames**: 4-byte **little-endian** length,
then that many bytes of JSON `tailcfg.MapResponse`. With `Compress:""`
frames are plain JSON. `{"KeepAlive": true}` frames are heartbeats and
carry no other data.

`MapResponse` essentials (delta semantics; nil field = "no change",
empty = "explicitly empty"): `Node` (self), `Peers` (full set replace),
`PeersChanged`, `PeersRemoved` (node IDs), `PeersChangedPatch`, `DERPMap`,
`DNSConfig`, `Domain`, `PacketFilter`, `UserProfiles`, `ControlTime`.

`tailcfg.Node` essentials: `ID`/`StableID`/`Name`/`User`, `Key`,
`KeyExpiry`, `Machine`, `DiscoKey`, `Addresses`, `AllowedIPs`,
`Endpoints`, `HomeDERP` (int region ID; the legacy `"DERP":
"127.3.3.40:N"` string form still appears from older servers and is
canonicalized to `HomeDERP`), `Hostinfo`, `Created`, `Online`,
`MachineAuthorized`, `Expired`.

## DERP relay protocol (Phase 3)

Ground truth: Go `derp/derp.go`, `derp/derp_client.go`,
`derp/derphttp/derphttp_client.go`. DERP relays opaque packets between nodes
that lack a direct path; it is also the disco side channel (Phase 5). The
**routing key is the node public key** (`key.NodePublic`, the WireGuard
static) — the same key WireGuard uses, so a node addresses a peer on DERP by
that peer's node key.

### Connection / HTTP upgrade (`derphttp`)

`GET /derp` with `Upgrade: DERP`, `Connection: Upgrade`. Server replies
`101 Switching Protocols` (headers include `Derp-Version: 2` and
`Derp-Public-Key: nodekey:…`). Production uses HTTPS; **Headscale's embedded
DERP also accepts the upgrade over plain HTTP on its `server_url` port**
(verified: region 999 on `:8080`), so Phase 3 needs no separate TLS relay.

### Frames

`[1B frameType][4B BE length][payload]`. `MaxPacketSize = 64 KiB`.

| type | name | payload |
|------|------|---------|
| 0x01 | ServerKey | 8B magic `DERP🔑` (`44 45 52 50 f0 9f 94 91`) + 32B server node pub |
| 0x02 | ClientInfo | 32B client node pub + 24B nonce + NaCl-box(json) |
| 0x03 | ServerInfo | 24B nonce + NaCl-box(json) |
| 0x04 | SendPacket | 32B dst node pub + packet |
| 0x05 | RecvPacket | (v2) 32B src node pub + packet |
| 0x06 | KeepAlive | none |
| 0x08 | PeerGone | 32B peer node pub + 1B reason |
| 0x09 | PeerPresent | 32B peer node pub + … |
| 0x12 | Ping | 8B opaque (echo in Pong) |
| 0x13 | Pong | 8B opaque |
| 0x14 | Health | UTF-8 problem string (empty = healthy) |
| 0x15 | Restarting | 2×4B BE (reconnect hints) |

### Handshake

1. Read ServerKey → server node public key.
2. Send ClientInfo: 32B our node public key, then a **NaCl box** (`crypto_box`
   = X25519 + XSalsa20-Poly1305) of the JSON `{"version":2,"CanAckPings":…}`
   sealed to the server key with our node private key. Box wire form is
   `24B random nonce || ciphertext+tag` (matches Go `key.NodePrivate.SealTo`).
3. Read ServerInfo (NaCl box, openable with our key) — validated, contents
   unused in Phase 3.

Only ClientInfo/ServerInfo are boxed; **relayed packets (Send/Recv) are
opaque** — DERP never sees inside the WireGuard payload.

### Rust decisions

- NaCl box via the RustCrypto `crypto_box` crate (`SalsaBox`) — DERP's
  control frames require exactly this construction; hand-rolling
  XSalsa20-Poly1305 is not worth the risk. Only the handshake needs it.
- DERP over plain TCP for Phase 3 (Headscale embedded DERP). `rustls` for
  HTTPS DERP arrives with the direct-path/hosted milestones.

## WireGuard data plane (Phase 3)

- **boringtun** `Tunn` per peer: node private key = WG static; peer node
  public key = peer static. `encapsulate(ip_pkt) → wg_pkt` and
  `decapsulate(wg_pkt) → ip_pkt`; a periodic `update_timers` tick drives
  handshakes/keepalives. Output is normally UDP-bound; in DERP-only mode
  every `wg_pkt` is carried in a DERP SendPacket frame keyed by the peer
  node key, and inbound RecvPacket frames are fed straight to
  `decapsulate`. WireGuard is oblivious to the path — the "magic socket"
  abstraction, minus path selection (Phase 5).
- No TUN in Phase 3 (that's Phase 4): the engine runs a tiny **userspace
  ICMP echo** responder/initiator over the tunnel so two nodes can prove
  relayed connectivity by pinging each other's `100.64.x.y` without root.

## TUN, routes, MagicDNS (Phase 4)

- **TUN device** (`ts-tun`): created with pure ioctls on `/dev/net/tun`
  (`TUNSETIFF`, `IFF_TUN | IFF_NO_PI`), then configured via a datagram socket
  (`SIOCSIFADDR`, `SIOCSIFNETMASK`, `SIOCSIFMTU`, `SIOCSIFFLAGS`). No
  `iproute2` dependency. Assigning the tailnet address with a **`/10`
  netmask** makes the kernel auto-install the connected route
  `100.64.0.0/10 dev ts0`, so no route command is needed. MTU 1280
  (Tailscale's default) avoids fragmentation over WireGuard+DERP. Async I/O
  via tokio `AsyncFd` on the non-blocking fd; one packet per `read`.
- **Checksum offload**: packets the local kernel routes into a TUN can carry
  an incomplete transport checksum (`CHECKSUM_PARTIAL` — only the
  pseudo-header sum), expecting hardware to finish it. The engine recomputes
  the TCP/UDP checksum in userspace before relaying (`ts-engine::l4`). ICMP
  is unaffected (always fully summed), which is why ping needs no fixup but
  TCP does on offloading stacks.
- **MagicDNS** (hosts stub): the engine renders netmap peer `DNSName`s +
  tailnet IPs into a managed block (delimited by `# BEGIN/END tailscale-rs
  MagicDNS`) in a hosts file, rewritten idempotently. A resolver on
  100.100.100.100:53 is a later refinement.
- **Proactive handshake**: on learning a peer from the netmap the engine
  initiates the WireGuard handshake immediately (encapsulating an empty
  packet), so the first real connection doesn't drop packets waiting for the
  handshake — matching tailscaled.

## Direct paths: disco, STUN, magicsock (Phase 5)

Ground truth: Go `disco/disco.go`, `net/stun/stun.go`, `wgengine/magicsock`.

### STUN (RFC 5389, `ts-stun`)

Binding request: `00 01`, 2-byte length, magic cookie `21 12 A4 42`, 12-byte
transaction id (we send the minimal form — no SOFTWARE/FINGERPRINT). The
success response (`01 01`) carries `XOR-MAPPED-ADDRESS` (attr `0x0020`):
port XORed with `0x2112`, address XORed with the cookie (+ txid for IPv6).
Gives the node its server-reflexive (public) endpoint for NAT traversal.

### disco (`ts-disco`)

Wire: `magic "TS💬" (54 53 f0 9f 92 ac)` + 32B sender disco public key +
24B nonce + NaCl box. The box is `crypto_box` (X25519 + XSalsa20-Poly1305)
between the two disco keys; plaintext is `msgType(1) || version(1) || data`.
Only the ClientInfo-style header key is cleartext, so a receiver can route
before decrypting. Message types:
- **Ping** (`0x01`): 12B tx id + 32B node key. Probes a candidate path.
- **Pong** (`0x02`): 12B tx id + 16B src IP (v4-mapped) + 2B port. Echoes
  the tx id and reports the source the ponger saw (so the pinger learns its
  reflexive address).
- **CallMeMaybe** (`0x03`): N × (16B IP + 2B port). "Here are my endpoints —
  ping me." Sent over DERP to bootstrap hole punching.

### magicsock (`ts-magicsock`)

One UDP socket multiplexing direct endpoints and DERP. Per-peer typed path
state (`Relay` ↔ `Direct(addr)`). Receive path classifies each datagram:
disco magic → handle disco; else → WireGuard (mapped to a peer by source
endpoint). Discovery/upgrade:
1. Report local + STUN-reflexive endpoints to control (see below), which
   propagates them + our disco key to peers.
2. Send **CallMeMaybe** over DERP with our endpoints (re-sent every 2 s until
   a direct path is up — the first can race the peer's DERP registration).
3. On CallMeMaybe, disco-**Ping** each of the peer's endpoints over UDP.
4. On an inbound Ping, **Pong** to its source and ping that source back
   (verify both directions); on a **Pong** matching one of our pings, mark
   that endpoint verified and **migrate** the peer's WireGuard traffic from
   DERP to direct. Heartbeat re-pings keep NAT mappings alive; a path with no
   pong for 15 s falls back to DERP. The WireGuard session is untouched by
   the migration.

### Control interaction (the disco-key gotcha)

The control server only advertises a node's disco key to peers **after the
node reports its endpoints**. Headscale ignores endpoints/disco on the
long-poll (streaming) map request — they must be sent via a separate
**lite** map request (`Stream=false`, `OmitPeers=true`). Without it, peers
receive a zero disco key and NAT traversal never starts. (Discovered
empirically; `ts-control::update_endpoints` sends the lite update at
startup.)

### Rust decisions

- STUN and disco reuse `crypto_box` (already in the tree for DERP). No new
  crypto dependency.
- magicsock is single-task-owned (no locks); the UDP socket is shared via
  `Arc` so the engine's `select!` loop can receive while magicsock sends.
- Verified on a flat L2 (both nodes' local endpoints reachable): two nodes
  upgrade DERP→direct and carry real TCP over the direct path, WG session
  intact. Two-distinct-NAT hole punching (esp. symmetric NAT — Linux
  MASQUERADE) is the empirically-hard case the plan flags; `interop/
  stun_server.py` and the `--stun` path exist for that harness.

## Packet filter (ACL) + LocalAPI (Phase 6)

### Packet filter in the netmap (`ts-filter`)

The control server compiles the tailnet ACL into a flat allow-list and ships
it in the `/machine/map` response. Two wire shapes exist:

- **Legacy**: `MapResponse.PacketFilter` — a flat `[]FilterRule`.
- **Modern (Headscale)**: `MapResponse.PacketFilters` — a **named map**,
  e.g. `{"base": [rule, …]}`. The effective filter is the union of all named
  sets. Captured from a live Headscale (no ACL policy):

  ```json
  "PacketFilters": {"base": [
    {"SrcIPs": ["*"], "DstPorts": [{"IP": "*", "Ports": {"First": 0, "Last": 65535}}]}
  ]}
  ```

`FilterRule` = `{SrcIPs: []string, DstPorts: [{IP, Ports:{First,Last}}],
IPProto: []int}`. `SrcIPs`/`IP` are CIDRs, bare IPs, or `"*"`. A packet is
**allowed** iff some rule matches: source in one of `SrcIPs`, destination in
one of `DstPorts`, and — if `IPProto` is non-empty — the protocol is listed.

- **Port-bearing protocols** (TCP=6, UDP=17, SCTP=132): the destination port
  must fall in `[First, Last]`.
- **Port-less protocols** (ICMP=1, …): matched only by a **full-range**
  (`0–65535`) destination — Go's compiler treats "all ports" as "any
  protocol", so a narrow TCP rule does not leak ICMP. Verified end-to-end:
  with a Headscale `accept tcp *:8000` policy, ICMP between two rusty nodes is
  dropped; the default allow-all passes it.

Enforcement is **inbound-only** (what peers may send us), checked in the
engine on each decrypted packet before it reaches the TUN; the engine starts
`allow_all` so startup isn't black-holed, and an empty ruleset is deny-all.
The netmap also carries `UserProfiles: []UserProfile` (for the status owner
column) — a delta field, accumulated across frames.

### LocalAPI (`ts-localapi`)

HTTP/1.1 over a Unix socket, byte-for-byte the endpoints the Go CLI uses:

| Method + path | Request | Response |
|---|---|---|
| `GET /localapi/v0/status` | — | `ipnstate.Status` (JSON) |
| `PATCH /localapi/v0/prefs` | `ipn.MaskedPrefs` (each field paired with a `<Field>Set` mask flag) | `ipn.Prefs` |
| `POST /localapi/v0/ping?ip=…&type=disco` | — | `ipnstate.PingResult` |

The `Host` header is `local-tailscaled.sock`; on Linux the Go daemon
authenticates by socket peer credentials — we bind the socket `0o600` as a
first line of defence until that check lands. `WantRunning` (via
`MaskedPrefs`) toggles the data plane: `Stopped` drops user traffic but keeps
WireGuard handshakes alive so a down/up cycle doesn't re-handshake.

## Userspace netstack (`ts-net`, Phase 7)

No new wire protocol — this is where our own decrypted IP packets terminate in
a userspace TCP/IP stack instead of the OS kernel.

- **Data path.** The engine hands each decrypted inbound IP packet to smoltcp
  as a received frame (medium = IP, no Ethernet/ARP); smoltcp's TCP sockets
  produce reply packets, which the engine encapsulates in WireGuard and sends
  to the peer over the best path (direct or DERP) — the same pipeline the TUN
  uses. Checksums are computed/verified by smoltcp (`Checksum::Both`), so no
  offload fixup is needed on this path.
- **Addressing.** Our tailnet address is assigned with the `100.64.0.0/10`
  prefix, making all peers on-link; no gateway/route is required.
- **MTU** = 1280 (matches the TUN path), comfortably under the physical MTU
  after WireGuard + DERP framing.
- **Accept model.** A pool of smoltcp listen sockets (backlog) per bound port;
  when one leaves `LISTEN` (a SYN arrived) it is promoted to a connection and
  replaced, so the port keeps accepting. Each connection is a tokio
  `AsyncRead + AsyncWrite` stream over bounded channels (TCP backpressure when
  the app reads slowly).

## Fuzzing the wire parsers (Phase 7)

Every parser that consumes untrusted bytes has a `tests/fuzz_smoke.rs` that
feeds it hundreds of thousands of pseudo-random and structure-mutated inputs
and asserts it neither panics nor over-allocates: STUN responses
(`ts-stun`), the disco receive path (`is_disco`/`source_key`/`open`,
`ts-disco`), and the DERP frame reader (`read_frame`, `ts-derp`), alongside
the existing panic-free tests for the IPv4/ICMP and ACL parsers. They run on
stable via a deterministic xorshift PRNG (reproducible from the seed); the
entry points are exactly what a `cargo-fuzz`/libfuzzer target would drive.

## Rust implementation decisions

- **Hand-rolled Noise IK** over `x25519-dalek` + `chacha20poly1305` +
  `blake2`/`hkdf`/`hmac`, mirroring `controlbase` exactly. `snow` was
  evaluated per the plan and rejected: the handshake matches the standard
  IK pattern, but the transport phase uses Tailscale's custom framing and
  big-endian record nonces, so snow's transport is unusable and its value
  shrinks to ~100 lines of handshake we'd still have to frame ourselves
  (via an escape-hatch raw split). Byte-exact interop is verified against
  the real Go `controlbase` package (see `interop/noise-server-go/`).
- HTTP/2 over the Noise conn via the `h2` crate (the hyper project's h2
  implementation, same role as Go's `x/net/http2`).
- Base64 for the handshake header is encode-only and hand-rolled (20
  lines, tested), consistent with the no-dep-for-two-functions policy.
