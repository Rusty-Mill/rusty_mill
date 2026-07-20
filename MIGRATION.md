# Migrating croc to Rust

This repository is a Rust port of [schollz/croc](https://github.com/schollz/croc)
(v10, ~8k lines of Go excluding tests) — a tool for securely sending files
between two computers via a code phrase, using a relay when the peers cannot
connect directly.

The port's guiding constraint is **wire compatibility**: a rusty-croc relay
must serve stock Go croc clients, and (eventually) a rusty-croc client must
talk to stock relays and stock peers. That constraint drives most of the
design decisions below, and it is already verified end-to-end for the relay
(see [Verification status](#verification-status)).

## croc's architecture (what we're porting)

| Go package | Lines | Role |
|---|---|---|
| `src/croc` | ~3,000 | Client engine: send/receive state machine, chunked transfers, resume, local-network discovery |
| `src/cli` | ~840 | CLI (urfave/cli): flags, config persistence, code-phrase entry |
| `src/tcp` | ~780 | Relay server: PAKE-authenticated rooms, socket stapling, multi-port transfers |
| `src/utils` | ~890 | Hashing (xxhash/imohash/highwayhash), file walking, IP discovery, misc |
| `src/comm` | ~220 | Framed TCP messaging (also SOCKS5/HTTP proxy dialing) |
| `src/message` | ~90 | JSON control-message envelope (compressed + encrypted) |
| `src/crypt` | ~125 | PBKDF2 + AES-256-GCM; Argon2id + XChaCha20-Poly1305 |
| `src/compress` | ~55 | Raw DEFLATE via `compress/flate` |
| `src/mnemonicode` | ~400 | Bytes → memorable words for code phrases (vendored, MIT) |
| `src/models` | ~170 | Constants, default relay resolution (incl. custom DNS) |
| `src/diskusage`, `src/install`, `src/message` | misc | Platform helpers |

Key third-party dependencies and their protocol relevance:

* **`schollz/pake/v3`** — SPAKE2-style PAKE (Boneh–Shoup fig. 21) over a
  pluggable curve. Wire format is JSON of the public struct fields, whose
  names contain Unicode subscripts (`Uᵤ`, `Xᵥ`, …) and whose coordinates are
  arbitrary-precision decimal JSON numbers (Go `big.Int`).
* **`tscholl2/siec`** — a nonstandard 255-bit "super-isolated" elliptic curve
  (y² = x³ + 19 over a 255-bit prime, generator (5,12)). **The relay
  handshake hardcodes this curve**, so no Rust port can interop with stock
  croc without implementing SIEC — no crates.io implementation exists. The
  peer-to-peer PAKE defaults to `p256` and negotiates the curve in-band
  (recipient's choice, sent alongside its PAKE message).

## The wire protocol (as implemented by croc v10)

### Framing (`comm`)

Every message on every socket (except raw piped transfer bytes) is framed:

```
b"croc" | u32 little-endian payload length | payload
```

Reads are guarded by a 64 MiB max size, a 3 h idle deadline and a 10 min
body deadline.

### Relay handshake (`tcp`)

1. Client → relay: PAKE role-0 message over **siec** with the fixed weak key
   `[1,2,3]`; relay answers with its role-1 message. Both derive
   `strongKey = SHA-256(pw ‖ X ‖ Y ‖ Z)`.
   (Alternatively a client sends the literal frame `ping` and gets `pong` —
   that's the health check.)
2. Client → relay: 8-byte salt; both sides compute
   `key = PBKDF2-HMAC-SHA256(strongKey, salt, 100 iters, 32 bytes)`.
   All further frames on this socket are `AES-256-GCM(12-byte nonce ‖ ct ‖ tag)`.
3. Client → relay: encrypted relay password (default `pass123`; this is
   access control for the relay, not transfer security). Relay → client:
   `"<banner>|||<client-external-ip>"`, where the banner on the main port
   lists the extra transfer ports (e.g. `9010,9011,9012,9013`).
4. Client → relay: room name (croc uses the first characters of the code
   phrase). First occupant gets `ok` and then a framed `[1]` keep-alive every
   second. When the second occupant joins, the relay sends it `ok` and
   staples the sockets — from then on it pipes raw bytes both ways and the
   clients speak directly to each other (encrypted end-to-end with keys the
   relay never learns).

### Peer protocol (over the stapled connection)

1. Peers run a second PAKE using the code phrase (minus the room prefix) —
   default curve `p256`, recipient picks and announces the curve. From the
   session key they derive the transfer key (PBKDF2, salt exchanged in the
   PAKE messages).
2. Control messages are `message.Message` JSON
   (`{"t","m","b","b2","n"}`, byte fields base64) → DEFLATE → AES-256-GCM,
   covering: `pake`, `externalip`, `fileinfo`, `recipientready`, `finished`,
   `error`, `close-*`.
3. File data flows over N parallel connections (the extra relay ports), in
   chunks: `u64 LE file position ‖ chunk data`, encrypted, with per-file
   hashing (xxhash default; imohash/highwayhash options) for resume support.

## Rust module mapping

| Go | Rust | Status | Notes |
|---|---|---|---|
| `src/comm` | `src/comm.rs` | ✅ ported | Frame-byte compatible (unit-tested against exact Go frame bytes). Proxy dialing deferred. |
| `src/crypt` | `src/crypt.rs` | ✅ ported | Verified against Go-generated PBKDF2/AES-GCM vectors. |
| `src/compress` | `src/compress.rs` | ✅ ported | `flate2`; decodes Go's HuffmanOnly output (vector-tested). Streams are mutually decodable, not byte-identical — that's fine, DEFLATE is DEFLATE. |
| `src/message` | `src/message.rs` | ✅ ported | JSON field names/base64/omitempty match Go exactly (vector-tested). |
| `src/mnemonicode` | `src/mnemonicode.rs` | ✅ ported | Same 1633-word list + algorithm, vector-tested. |
| `schollz/pake/v3` | `src/pake.rs` | ✅ ported | All four curves incl. SIEC, written against Go-generated curve vectors; live-tested against Go in both roles. `ed25519` option not yet ported (croc never defaults to it). |
| `src/tcp` | `src/tcp.rs` | ✅ ported | Relay serves stock Go croc clients (verified end-to-end); client-side `connect_to_tcp_server` verified against the stock Go relay. |
| `src/models` | `src/models.rs` | ✅ constants | Custom-DNS default-relay resolution deferred. |
| `src/utils` | `src/utils.rs` | ✅ core ported | Code phrases, all four hash algorithms (xxhash/imohash/highway/md5, vector-tested), chunk ranges, open-port scan, local IPs. |
| `schollz/peerdiscovery` | `src/discovery.rs` | ✅ ported | UDP multicast announce/discover (IPv4; same wire format, same self-filter semantics). IPv6 group pending. |
| `src/croc` | `src/croc.rs` | ✅ ported | Send/receive engine incl. the local path: auto-started local relay, multicast announce, `ips?` probe hand-off, `--ip` direct, zip mode, text mode, throttling. Reconnect-after-drop pending (phase 4). |
| `src/cli` | `src/main.rs` | 🟡 mostly | `send` (files/folders/text/stdin/zip/throttle), `receive`, `relay`, `ping`. Pending: `--remember`, QR, proxies, excludes, `--git`. |
| `src/diskusage`, `src/install` | — | ⬜ later | Platform niceties, not protocol. |

### Crate choices

* **Crypto**: RustCrypto (`aes-gcm`, `chacha20poly1305`, `pbkdf2`, `argon2`,
  `sha2`) — mature, pure-Rust, parameter-compatible with `golang.org/x/crypto`.
* **Curve math for PAKE**: `num-bigint` with a generic affine short-Weierstrass
  implementation. Rationale: (a) SIEC exists in no Rust crate, so bignum math
  is required anyway; (b) the PAKE wire format serializes raw affine
  coordinates as decimal, which fights the RustCrypto curve APIs; (c) it
  mirrors what Go does (`math/big` via `crypto/elliptic`'s legacy interface —
  also not constant-time in the SIEC path). Hardening note below.
* **Concurrency**: std threads, mirroring croc's goroutine structure 1:1.
  The relay is I/O-light (a few sockets per transfer); async brings no win at
  this scale and a large complexity tax during a port whose main risk is
  protocol divergence. Revisit tokio in phase 4 if relay fan-out matters.
* **CLI**: `clap` (derive) in place of `urfave/cli`.
* **JSON**: `serde_json` with `arbitrary_precision` (required: PAKE
  coordinates are >64-bit JSON numbers; without the feature they'd be lossily
  parsed as `f64`).

### Compatibility gotchas discovered while porting

These are the traps for anyone continuing this migration:

1. **SIEC is mandatory** for relay interop (hardcoded in `tcp.go`), even
   though peers default to p256.
2. **Go `big.Int` marshals as a bare decimal JSON number** of arbitrary size;
   `serde_json` needs `arbitrary_precision` or values silently round-trip
   through `f64` and corrupt.
3. **Go `[]byte` marshals as standard-alphabet padded base64 strings** in
   `message.Message`.
4. **`big.Int.Bytes()` is minimal big-endian and empty for zero** — the PAKE
   session-key hash transcript depends on this exact encoding.
5. **`omitempty` semantics** must be mirrored field-by-field or JSON
   comparisons (and hashes over JSON) diverge.
6. **The PAKE struct's JSON keys contain Unicode subscript characters**
   (`Uᵤ`); serde derive + rename handles it, but hand-rolled serializers must
   emit them byte-exactly.
7. **The relay keep-alive `[1]` frames** interleave with the handshake on the
   first occupant's socket; clients must skip them, and the relay must stop
   sending them under the same lock that staples the room, or a stray `[1]`
   corrupts the piped stream.
8. **PBKDF2 with 100 iterations** (not a typo — croc's choice, presumably for
   throughput on the already-high-entropy PAKE output) and **8-byte salts**.
9. Go's `elliptic.ScalarMult` semantics (scalar not pre-reduced) are safe to
   replicate with plain double-and-add because all four curves have prime
   order — reduction cannot change the result.

## Verification status

Everything below runs in `scripts/interop_test.sh` (needs Go + the croc
source) or `cargo test` (self-contained):

* ✅ `cargo test` — 38 unit tests, including Go-generated vectors for:
  PBKDF2 keys, AES-GCM decryption, DEFLATE decompression of Go output,
  message envelope decode (plain + encrypted), mnemonicode encodings,
  scalar/add/double results on all four curves, xxhash/md5 file hashes,
  and the FileInfo/SenderInfo/RemoteFileRequest/SimpleMessage JSON shapes.
* ✅ **Stock Go croc binary transfers a 1 MiB file through the rusty-croc
  relay** (5 ports, parallel transfer connections), checksums equal.
* ✅ Rust client handshake (`connect_to_tcp_server`) joins a room on the
  stock Go relay and reads the banner.
* ✅ **Transfer matrix through the rusty-croc relay**: rust→rust,
  rust→go, and go→rust file transfers, checksums equal. Folder transfers
  (nested + empty dirs) and identical-file skip verified in both
  directions during development.
* ✅ **Local-network route**: a stock Go recipient's `ips?` probe hops
  onto the rusty-croc sender's auto-started local relay (and vice versa —
  the Rust recipient hops onto Go's); `--ip` direct connections verified
  both directions; multicast announce/discover unit-tested over loopback.
* ✅ **`--text` both directions**, **`--zip` go→rust** (auto-unpacked),
  `--throttle` rate limiting, and a live identical-file skip with
  `--hash imohash` across implementations.

## Roadmap

### Phase 1 — foundations + relay (done)

See the mapping table.

### Phase 2 — the file-transfer engine (done: core)

`src/croc.rs` ports the transfer state machine: peer PAKE with curve
negotiation and salt/PBKDF2 key derivation, the optional pake1/ips?
handshake probe (answered for stock recipients doing local discovery),
`fileinfo`/`recipientready` exchange, parallel chunked transfer striped
round-robin over the relay's transfer ports (`u64 LE position ‖ data`,
per-chunk DEFLATE + AES-GCM), missing-chunk resume, folder + empty-folder
+ symlink handling, and the close/finished handshakes.

Notable divergences (documented in code): `ModTime` is sent as Go's zero
time (peers then skip their optional chtimes on skipped files), reconnect
support is declared as version 0 (Go peers fall back to no-reconnect),
and `imohash`/`highway` hash options aren't ported yet.

### Phase 3 — local-network path + everyday features (done: core)

* **Local path**: the sender auto-starts a relay on open ports (0.0.0.0),
  announces `croc<port>` over UDP multicast (`src/discovery.rs`, same wire
  format as `peerdiscovery`), and races the local route against the remote
  relay. The recipient tries multicast discovery, then the `ips?` probe —
  a SimpleMessage PAKE + encrypted query over the relay that returns the
  sender's `[local-port, ip...]` — and hops onto the sender's local relay
  when reachable. `--ip` connects straight to a sender. Verified against
  stock croc in both directions (the probe hand-off is croc's own
  same-host/TestFlag path; multicast between distinct hosts is
  unit-tested via loopback).
* **`--text` / stdin**: temp `croc-stdin-*` files with `SendingText`,
  printed on arrival; incoming text files get random local names exactly
  like Go (this matters — without the rename, a same-directory transfer
  self-skips).
* **`--zip`**: folders zipped (stored, base-name-prefixed entries like
  `utils.ZipDirectory`), sent as `TempFile`, auto-unpacked and removed on
  the receiving side.
* **`--throttle`**: token-bucket rate limit shared across data threads.
* **imohash/highway**: full hash-algorithm parity, vector-tested and
  live-tested (identical-file skip with `--hash imohash` across
  implementations).

Still pending from this phase: IPv6 multicast group, custom
`--multicast` address.

### Phase 4 — reconnect, CLI tail, hardening

* Reconnect-and-resume on dropped relays (ReconnectVersion 1 rooms).
* CLI parity: code-phrase prompt, `--remember` config file, QR code,
  proxy dialing (SOCKS5/HTTP `CONNECT`), custom-DNS relay resolution,
  exclude patterns, `--git` mode.

### Phase 5 — hardening & divergence budget

* Constant-time curve arithmetic for p256/p384/p521 via RustCrypto crates
  (keep bignum SIEC, or upstream a constant-time SIEC — Go's is also
  variable-time, so this exceeds upstream rather than lagging it).
* Zeroize key material; audit nonce handling.
* Fuzz the frame/message/PAKE parsers (`cargo-fuzz`; the Go side has fuzz
  corpora to borrow).
* Property tests cross-checking Go and Rust binaries in CI.
* Evaluate async for relay scalability; evaluate `croc`'s newer features as
  upstream moves (this port tracks v10.2.x behavior).

## Building and testing

```sh
cargo build --release            # produces target/release/rusty-croc
cargo test                       # self-contained unit + vector tests
scripts/interop_test.sh          # full Go↔Rust interop (needs Go toolchain)

target/release/rusty-croc relay  # start a relay stock croc clients can use
target/release/rusty-croc ping 127.0.0.1:9009
```
