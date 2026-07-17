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
| Crypto primitives | `crypto` | Hand-rolled MD4, MD5, SHA-1, SHA-256, HMAC-MD5/SHA-1, RC4, AES, PBKDF2, and a minimal bignum for RSA — no crypto crate. |
| NTLM | `ntlm` | NTLMv2 authentication (MS-NLMP): NEGOTIATE/CHALLENGE/AUTHENTICATE messages, the NTLMv2 response and key schedule, and the extended-session-security sealing used by CredSSP. |
| CredSSP / NLA | `credssp` | The `TSRequest` DER exchange (MS-CSSP): NTLM tokens, the public-key channel binding (SHA-256 nonce hash, or legacy), and sealed credential delegation. Pure codec + crypto, driven over TLS by the `tls` feature. |
| Kerberos | `krb5` | Kerberos v5 (RFC 4120 / MS-KILE): the RC4-HMAC (etype 23) and AES (etypes 17/18, RFC 3962) encryption profiles, the ASN.1 building blocks, the message PDUs (`Ticket`, `Authenticator`, `AP-REQ`, the AS/TGS exchange, `KRB-ERROR`), and the GSS-API/SPNEGO wrapping (`krb5::gss`) that carries the AP-REQ in CredSSP `negoTokens`. The KDC transport and RFC 4121 message sealing are still to come. |
| Client Info | `client_info` | `TS_INFO_PACKET` logon data (domain/user/password/shell, extended info). |
| Licensing | `license` | Licensing preamble and the License Error Message (`STATUS_VALID_CLIENT` detection). |
| Session framing | `pdu` | Share Control / Share Data headers with the `PDUTYPE` / `PDUTYPE2` constants. |
| Capabilities | `capabilities` | Demand Active / Confirm Active PDUs and the core capability sets (general, bitmap, pointer, input, share; others preserved raw). |
| Finalization | `finalization` | Synchronize / Control / Font List / Font Map PDUs and the client finalization sequence. |
| Input | `input` | Client Input Event PDU with scancode / Unicode / mouse / extended-mouse / sync events. |
| Output | `output` | Server graphics Update PDUs: bitmap (rectangles + verbatim data stream), palette, synchronize; orders kept raw. |
| Pointer | `pointer` | Server cursor updates: system / position / color / new / cached, with `ColorPointer::to_rgba()` for cursor rendering. |
| Fast-path | `fastpath` | The compact framing servers use for most traffic: output update parsing (bitmap/palette/pointer) and tight input event encoding. |
| Bitmap RLE | `rle` | The interleaved RLE bitmap decompressor (8/15/16/24 bpp), reachable via `BitmapData::decompressed()`. |
| Pixel unpack | `pixel` | Native pixel formats (8 indexed / 15 / 16 / 24 / 32 bpp) → top-down RGBA8888, via `BitmapData::to_rgba()`. |
| Framebuffer | `display` | RGBA desktop surface with clipped blit, `apply_bitmap`, and a PPM dump; assembles server bitmap updates. |
| TCP driver | `net` | Blocking `RdpTransport<S>` with `establish()` — the full standard-RDP bring-up (negotiation → MCS → security → logon → licensing → capabilities → finalization) — plus `establish_enhanced()` for the TLS path, the individual steps, and secure I/O-channel send/recv. The one module that touches a socket. |
| TLS connector | `tls` | *(optional `tls` feature)* `connect_tls()` — upgrades the TCP stream to TLS with `rustls` and drives `establish_enhanced()`. The crate's only third-party dependency, and off by default. |
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

The enhanced-security (TLS) path is also wired up: `RdpTransport::negotiate()`
selects `SSL` on the raw TCP connection, the stream is upgraded to TLS, and
`RdpTransport::establish_enhanced()` drives MCS/GCC, logon, licensing,
capabilities and finalization inside the tunnel with the RDP security layer
switched off (no Security Exchange, no RC4 — TLS carries confidentiality). The
protocol logic for this lives entirely in the dependency-free core; the actual
TLS bytes are the one thing behind an optional feature.

CredSSP / NLA (the `HYBRID` path) is implemented too: NTLMv2 authentication,
the CredSSP `TSRequest` exchange with the public-key channel binding, and
sealed credential delegation — all in the dependency-free core (`ntlm`,
`credssp`), verified against the published MS-NLMP test vectors. With the `tls`
feature, `connect_tls()` runs the whole exchange over the TLS channel when the
server selects `HYBRID`. NTLMv2 is the fully wired authentication mechanism;
Kerberos (the `krb5` module) is being built bottom-up — the RC4-HMAC crypto
profile, the ASN.1 foundations, and the message PDUs (AP-REQ, the AS/TGS
exchange, and the error reply) are in place, with the KDC transport, AES
encryption types, and SPNEGO/GSS wrapping still to come.

> **Security note:** the `crypto` and `security` modules implement obsolete,
> deliberately weak algorithms (RC4, MD5/SHA-1 MACs, unpadded RSA) purely to
> speak RDP *standard security*. They are not for general use; modern
> deployments should negotiate TLS/CredSSP. The `tls` feature's `connect_tls()`
> does **not** verify the server certificate (RDP servers are typically
> self-signed with out-of-band trust), so it does not defend against an active
> man-in-the-middle — bring your own verified `rustls` stream and use
> `establish_enhanced()` if you need that.

## Design principles

- **No I/O in the codec.** The codec types encode to and decode from byte
  slices, so the same code works with blocking sockets, any async runtime, or
  in-memory tests. Socket access lives in exactly one module (`net`), a thin
  blocking driver kept apart from the codec.
- **Minimal dependencies.** The core has zero. The one thing that genuinely
  needs a third-party crate — a TLS stack, which cannot be hand-rolled
  responsibly — lives behind the optional `tls` feature (`rustls`), never in
  the default build. Even the RDP-over-TLS *protocol* logic stays in the
  dependency-free core: the transport is generic over the stream, so you can
  bring your own TLS implementation instead. RSA for standard RDP security is
  hand-rolled, so it needs no crate.
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
cargo build            # zero dependencies
cargo test
cargo build --features tls   # opt-in TLS connector (pulls in rustls)
```

The default build has no dependencies and keeps an MSRV of 1.70. The optional
`tls` feature pulls in `rustls` and its transitive crates, which raise the
effective MSRV to whatever `rustls` requires.

## Connecting to a server

The `connect` example drives the deterministic part of the connection sequence
(X.224 negotiation, MCS connect, channel setup) against a live server:

```sh
cargo run --example connect -- 192.0.2.10:3389 alice
```

`RdpTransport::establish()` drives the whole standard-RDP bring-up
(negotiation → MCS → security exchange → encrypted logon → licensing →
capabilities → finalization) and returns an active session. The example then
pumps server updates with `recv_event()` — which accepts both slow-path and
fast-path framing — into a `Framebuffer` until the stream goes quiet and writes
the result to `screen.ppm`; `send_input()` sends keyboard/mouse events over
fast-path.

For a server that requires TLS, enable the `tls` feature and use
`connect_tls()` instead, which negotiates `SSL`, upgrades to TLS, and runs
`establish_enhanced()`:

```rust
# #[cfg(feature = "tls")]
# fn demo() -> std::io::Result<()> {
use rusty_rdp::net::EstablishConfig;
use rusty_rdp::nego::SecurityProtocols;
use rusty_rdp::tls::connect_tls;

let config = EstablishConfig::new(1024, 768, "", "alice", "secret");
let (mut transport, session) =
    connect_tls("192.0.2.10:3389", &config, SecurityProtocols::SSL)?;
let _ = (transport.recv_event()?, session);
# Ok(())
# }
```

## License

Licensed under either of MIT or Apache-2.0 at your option.
