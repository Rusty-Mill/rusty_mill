# rusty_rdp

A minimal, **dependency-free** implementation of the Remote Desktop Protocol
(RDP) wire format in Rust.

The goal is a clean RDP codec built on nothing but the Rust standard library —
no `tokio`, no `openssl`, no third-party crates in the core at all. Every wire
structure is encoded and decoded by hand with bounds-checked cursors, and
`unsafe` is forbidden crate-wide.

## Status

Early foundation. Implemented so far, bottom-up:

| Layer | Module | What it does |
|-------|--------|--------------|
| TPKT (RFC 1006) | `tpkt` | 4-byte length framing over TCP, with stream-friendly `peek_total_len`. |
| X.224 Class 0 | `x224` | Connection Request / Confirm / Data TPDUs, cookie parsing. |
| RDP negotiation | `nego` | `RDP_NEG_REQ` / `RSP` / `FAILURE` security selection. |
| MCS (T.125) | `mcs` | `Connect-Initial` / `Connect-Response` and the domain PDUs (erect domain, attach user, channel join, send data). |
| GCC (T.124) | `gcc` | Conference Create Request/Response envelope and the typed `TS_UD_*` settings blocks (client/server core, security, network, cluster). |
| Standard security | `security` | Server certificate → RSA key, client-random encryption, the key-derivation schedule, the Security Exchange PDU, the basic security header, and RC4 + MAC for encrypted PDUs. |
| Crypto primitives | `crypto` | Hand-rolled MD5, SHA-1, RC4, and a minimal bignum for RSA — no crypto crate. |
| Client Info | `client_info` | `TS_INFO_PACKET` logon data (domain/user/password/shell, extended info). |
| Licensing | `license` | Licensing preamble and the License Error Message (`STATUS_VALID_CLIENT` detection). |
| Session framing | `pdu` | Share Control / Share Data headers with the `PDUTYPE` / `PDUTYPE2` constants. |
| Capabilities | `capabilities` | Demand Active / Confirm Active PDUs and the core capability sets (general, bitmap, pointer, input, share; others preserved raw). |
| Finalization | `finalization` | Synchronize / Control / Font List / Font Map PDUs and the client finalization sequence. |
| Input | `input` | Client Input Event PDU with scancode / Unicode / mouse / extended-mouse / sync events. |
| Output | `output` | Server graphics Update PDUs: bitmap (rectangles + verbatim data stream), palette, synchronize; orders kept raw. |
| Bitmap RLE | `rle` | The interleaved RLE bitmap decompressor (8/15/16/24 bpp), reachable via `BitmapData::decompressed()`. |
| Pixel unpack | `pixel` | Native pixel formats (8 indexed / 15 / 16 / 24 / 32 bpp) → top-down RGBA8888, via `BitmapData::to_rgba()`. |
| BER (X.690) | `ber` | The definite-length TLV subset the MCS connection PDUs need. |
| PER (X.691) | `per` | The ALIGNED-PER subset the MCS domain PDUs and GCC envelope need. |
| Byte cursors | `cursor` | Explicit big/little-endian, bounds-checked read/write. |

This is enough to build and parse the RDP connection sequence from the X.224
negotiation all the way through the logon and licensing exchange: the BER
`Connect-Initial`/`Response`, the GCC conference settings blocks, the PER
domain PDUs, the RSA/RC4 security handshake, the encrypted Client Info PDU, and
the licensing round trip, the Share Control / Share Data framing that every
session PDU rides in, the capability exchange (Demand / Confirm Active), the
connection-finalization sequence (synchronize, control, font list/map), and
client input events (keyboard and mouse), and the server-to-client display
path all the way to pixels: bitmap and palette updates, RLE decompression, and
pixel-format unpacking to a top-down RGBA framebuffer. Pointer/cursor updates
and fast-path framing build on top without disturbing what is here.

> **Security note:** the `crypto` and `security` modules implement obsolete,
> deliberately weak algorithms (RC4, MD5/SHA-1 MACs, unpadded RSA) purely to
> speak RDP *standard security*. They are not for general use; modern
> deployments should negotiate TLS/CredSSP (the `SSL`/`HYBRID` path), which is
> the next security milestone.

## Design principles

- **No I/O in the codec.** Types encode to and decode from byte slices, so the
  same code works with blocking sockets, any async runtime, or in-memory
  tests. Nothing in the crate opens a socket.
- **Minimal dependencies.** The core has zero. Anything that genuinely needs a
  third-party crate (TLS for the `SSL`/`HYBRID` security modes, RSA for
  standard RDP security) will live behind an optional feature flag, never in
  the default build.
- **Total decoding.** Malformed input returns an `Error`; it never panics.
- **Explicit endianness.** RDP mixes big-endian transport framing with
  little-endian RDP structures, so every integer access names its byte order.

## Example

```rust
use rusty_rdp::nego::{Negotiation, SecurityProtocols};
use rusty_rdp::tpkt::Tpkt;
use rusty_rdp::x224::X224;

// Client Connection Request asking for TLS or CredSSP.
let neg = Negotiation::Request {
    flags: 0,
    protocols: SecurityProtocols::SSL | SecurityProtocols::HYBRID,
};
let x224 = X224::connection_request(neg);
let tpdu = x224.to_vec().unwrap();
let packet = Tpkt::new(&tpdu).to_vec().unwrap();
// `packet` is ready to write to a TCP socket.
```

## Building

```sh
cargo build
cargo test
```

## License

Licensed under either of MIT or Apache-2.0 at your option.
