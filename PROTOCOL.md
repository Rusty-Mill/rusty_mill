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
