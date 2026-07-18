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
| CredSSP / NLA | `credssp` | The `TSRequest` DER exchange (MS-CSSP): the NTLM and Kerberos client state machines, the public-key channel binding (SHA-256 nonce hash, or legacy), and sealed credential delegation. Pure codec + crypto, driven over TLS by the `tls` feature. |
| Kerberos | `krb5` | Kerberos v5 (RFC 4120 / MS-KILE): the RC4-HMAC (etype 23) and AES (etypes 17/18, RFC 3962) encryption profiles, the ASN.1 building blocks, the message PDUs (`Ticket`, `Authenticator`, `AP-REQ`, the AS/TGS exchange, `KRB-ERROR`), the GSS-API/SPNEGO wrapping (`krb5::gss`) that carries the AP-REQ in CredSSP `negoTokens`, and the RFC 4121 per-message Wrap/MIC sealing (`krb5::cfx`). The KDC transport and CredSSP wiring are still to come. |
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
| TCP driver | `net` | Blocking `RdpTransport<S>` with `establish()` — the full standard-RDP bring-up (negotiation → MCS → security → logon → licensing → capabilities → finalization) — plus `establish_enhanced()` for the TLS path, `accept()` for the server side (see below), the individual steps, secure I/O-channel send/recv, and generic static-virtual-channel routing (`extra_channels`, `RdpEvent::ChannelData`, `send_channel_data`). The one module that touches a socket. |
| Virtual channel chunking | `vchan` | MS-RDPBCGR 2.2.6.1 `CHANNEL_PDU_HEADER` framing shared by every static virtual channel: splits outbound messages into chunks and reassembles inbound ones. What `net` uses to receive traffic on any channel beyond the I/O channel. |
| Dynamic virtual channels | `dvc` | MS-RDPEDYC PDU framing for the `DRDYNVC` channel that RDPGFX, clipboard, and other redirection protocols multiplex over: create/data/close, the version 1–3 capability exchange, and `fragment()` for outbound message splitting. |
| DVC session management | `dvcman` | `DvcManager` — tracks open dynamic channels, auto-accepts `Create` requests and echoes `Capabilities` requests, and reassembles a channel's own fragmented messages into `DvcEvent::{ChannelOpened, Data, ChannelClosed}`. The layer a caller drives with `net`'s `RdpEvent::ChannelData` to reach a named DVC-based protocol (e.g. RDPGFX) without hand-parsing `dvc` PDUs. |
| Graphics pipeline | `gfx` | MS-RDPEGFX PDUs carried over the `dvc`/`dvcman` `"Microsoft::Windows::RDS::Graphics"` channel: capability negotiation (`CapsAdvertisePdu`/`CapsConfirmPdu`), surface lifecycle (`CreateSurfacePdu`/`DeleteSurfacePdu`), the bitmap-carrying and frame-sequencing PDUs (`WireToSurface1Pdu`/`WireToSurface2Pdu`, `StartFramePdu`/`EndFramePdu`/`FrameAcknowledgePdu`), the bitmap cache PDUs (`SurfaceToCachePdu`/`CacheToSurfacePdu`/`EvictCacheEntryPdu`/`CacheImportOfferPdu`/`CacheImportReplyPdu`), surface composition (`SolidFillPdu`/`SurfaceToSurfacePdu`), output mapping (`ResetGraphicsPdu`, `MapSurfaceToOutputPdu`/`MapSurfaceToScaledOutputPdu`, `MapSurfaceToWindowPdu`/`MapSurfaceToScaledWindowPdu`), and the AVC420/AVC444 wrapper formats (`Avc420BitmapStream`/`Avc444BitmapStream`) — region and quantization metadata only, since decoding the H.264 bitstreams they carry needs an actual H.264 decoder. |
| RemoteFX codec | `rfx` | MS-RDPRFX tile decode for `gfx`'s `CODECID_CAVIDEO` bitmap data: RLGR1 and RLGR3 entropy decoding (`EntropyAlgorithm`), the 3-level 5/3 lifting-scheme inverse DWT, per-sub-band dequantization, and YCbCr→RGB, wired together by `Tile::decode_rgb`/`TileSet`, plus the control PDUs that wrap a tile set on the wire (`SyncPdu`, `CodecVersionsPdu`, `ChannelsPdu`, `ContextPdu`, `RegionPdu`, `FrameBeginPdu`/`FrameEndPdu`, dispatched by `peek_block_type`). The GFX cache/composition PDUs (`SURFACETOCACHE`, `SOLIDFILL`, etc.) belong to `gfx` instead, and encoding (the server-side direction) is not implemented. |
| Planar codec | `planar` | MS-RDPEGDI RDP 6.0 bitmap decode for `gfx`'s `CODECID_PLANAR` bitmap data: `decode()` turns an `RDP6_BITMAP_STREAM` into a top-down RGBA8888 buffer, handling all four color planes (alpha/luma-or-red/orange-or-green/green-or-blue), the scan-line delta RLE scheme (`RDP6_RLE_SEGMENT`), the optional AYCoCg color space with color-loss reduction and 2×2 chroma subsampling, and the documented decoder-side R/B swap. Decode-only, like `rfx`. |
| ClearCodec | `clearcodec` | MS-RDPEGFX decode for `gfx`'s `CODECID_CLEARCODEC` bitmap data: `ClearCodecDecoder` composites a `CLEARCODEC_BITMAP_STREAM`'s three payloads (a full-canvas run-length `residualData` fill, per-column `bandsData` "VBar" runs, and independent raw/RLEX `subcodecsData` sub-tiles) onto a top-down RGBA8888 buffer, and owns the persistent glyph and VBar/short-VBar caches later messages in the same session reference by index. NSCodec sub-tiles (`subcodecId == 1`) are not implemented. Decode-only, like `rfx`/`planar`. |
| Clipboard redirection | `cliprdr` | MS-RDPECLIP PDUs on the `"cliprdr"` static channel: the caps/monitor-ready handshake (`CapsPdu`/`GeneralCapabilitySet`/`MonitorReadyPdu`), format announcement (`FormatListPdu`/`FormatListResponsePdu`, Long Format Name variant), and data transfer (`FormatDataRequestPdu`/`FormatDataResponsePdu`, with `as_unicode_text()` for `CF_UNICODETEXT`). File copy/paste and the Short Format Name variant are not implemented. |
| Audio redirection | `rdpsnd` | MS-RDPEA PDUs on the `"rdpsnd"` static channel: format negotiation (`AudioFormatsPdu`/`AudioFormat`), bandwidth training (`TrainingPdu`/`TrainingConfirmPdu`), and wave playback (`encode_wave`/`decode_wave` hiding the WaveInfo/Wave PDU split, `WaveConfirmPdu`), plus `ClosePdu`. Volume/pitch control, `SNDC_WAVE2`, encryption, and the UDP transport variants are not implemented. |
| Device redirection | `rdpdr` | MS-RDPEFS PDUs on the `"rdpdr"` static channel: the full initialization/capability handshake (`ServerAnnounceRequestPdu`/`ClientAnnounceReplyPdu`/`ServerClientIdConfirmPdu`/`ClientNameRequestPdu`, `ServerCoreCapabilityPdu`/`ClientCoreCapabilityPdu` with `GeneralCapsSet`, `ClientDeviceListAnnouncePdu`/`ServerDeviceAnnounceResponsePdu`, `ServerUserLoggedOnPdu`), and the full Device I/O Request/Response exchange (`DeviceIoRequest`/`DeviceIoResponse` headers) for every major function but one: create/close/read/write (`DeviceCreateRequestPdu`/`RspPdu`, `DeviceCloseRequestPdu`/`RspPdu`, `DeviceReadRequestPdu`/`RspPdu`, `DeviceWriteRequestPdu`/`RspPdu`), the generic IOCTL/FSCTL carrier (`DeviceControlRequestPdu`/`RspPdu`) that smart-card and port redirection ride on, query/set file information (`QueryInformationRequestPdu`/`RspPdu`, `SetInformationRequestPdu`/`RspPdu`), query/set volume information (`QueryVolumeInformationRequestPdu`/`RspPdu`, `SetVolumeInformationRequestPdu`/`RspPdu`), and directory control — listing (`QueryDirectoryRequestPdu`/`RspPdu`) and change notification (`NotifyChangeDirectoryRequestPdu`/`RspPdu`). Lock control is not implemented — its request layout isn't in Microsoft's published spec pages, and no reference client (FreeRDP, rdesktop, xrdp) actually parses it either. |
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
server selects `HYBRID`. Both authentication mechanisms are wired end to end:
NTLMv2 (`connect_tls`) and Kerberos (`connect_tls_kerberos`, which takes a
caller-supplied ticket + AES session key and drives the SPNEGO/AP-REQ exchange
sealed with RFC 4121 Wrap tokens). The one remaining Kerberos gap is the KDC
transport that fetches the ticket in the first place.

> **Security note:** the `crypto` and `security` modules implement obsolete,
> deliberately weak algorithms (RC4, MD5/SHA-1 MACs, unpadded RSA) purely to
> speak RDP *standard security*. They are not for general use; modern
> deployments should negotiate TLS/CredSSP. The `tls` feature's `connect_tls()`
> does **not** verify the server certificate (RDP servers are typically
> self-signed with out-of-band trust), so it does not defend against an active
> man-in-the-middle — bring your own verified `rustls` stream and use
> `establish_enhanced()` if you need that.

## Roadmap

Known gaps versus full-featured implementations (FreeRDP, IronRDP), roughly in
the order they'd add the most value:

- ~~**RemoteFX / GFX codec support.**~~ Done. The channel plumbing (`vchan`,
  `dvc`, `dvcman`), the MS-RDPEGFX capability negotiation, surface/frame
  PDUs, the bitmap cache PDUs (`SurfaceToCachePdu`/`CacheToSurfacePdu`/
  `EvictCacheEntryPdu`/`CacheImportOfferPdu`/`CacheImportReplyPdu`), surface
  composition (`SolidFillPdu`/`SurfaceToSurfacePdu`), and output mapping
  (`ResetGraphicsPdu`, `MapSurfaceToOutputPdu`/`MapSurfaceToScaledOutputPdu`,
  `MapSurfaceToWindowPdu`/`MapSurfaceToScaledWindowPdu`) are all wired up in
  `gfx`. Every bitmap codec RDPGFX carries is now handled: the RemoteFX
  tile codec (`rfx` — RLGR1/RLGR3 entropy decoding, the 3-level 5/3 inverse
  DWT, dequantization, and YCbCr→RGB, plus its control PDUs), the RDP 6.0
  Planar codec (`planar` — scan-line delta RLE, AYCoCg with color-loss
  reduction and chroma subsampling), and ClearCodec (`clearcodec` —
  residual/bands/subcodec compositing plus the persistent glyph and VBar
  caches it depends on, except its NSCodec sub-tile variant, a whole
  separate legacy codec out of scope on its own) all decode straight to
  pixels. AVC420/AVC444 (`gfx::Avc420BitmapStream`/`Avc444BitmapStream`)
  parse their region/quantization metadata and hand back the raw H.264
  Annex B bitstream unopened — actually decoding it needs a real H.264
  decoder, permanently out of scope for a dependency-free crate.
- **Channels: drive, USB, smartcard, and printer redirection.** The
  static/dynamic virtual channel plumbing all of these ride on (`vchan`,
  `dvc`, `dvcman`, and `net`'s generic channel routing) is implemented end
  to end, and clipboard redirection (CLIPRDR, `cliprdr`, text only), audio
  redirection (RDPEA, `rdpsnd`, PCM/format negotiation and wave playback),
  and RDPDR (`rdpdr`) are now implemented, essentially completely: the
  initialization/capability handshake and the full Device I/O
  Request/Response exchange — create/close/read/write, generic device
  control (the IOCTL/FSCTL carrier smart-card and port redirection ride on
  almost entirely), query/set file and volume information, and directory
  listing/change notification. Still needed: clipboard file copy/paste,
  audio volume/pitch/encryption, and RDPDR's lock control (its request
  layout isn't published anywhere findable, and no reference client
  implementation actually parses it either — low priority given that).
- **Server-side RDP.** `RdpTransport::accept` now drives the server half of
  the connection sequence too — X.224 Connection Confirm, GCC/MCS
  Connect-Response, channel setup, Client Info, "no license required",
  Demand Active/Confirm Active, and the server's finalization sequence —
  reusing the same bidirectional codec types `establish*` uses, and tested
  end to end over a real TCP loopback connection against a hand-driven
  client. It's restricted to **unencrypted** standard RDP security
  (`encryptionLevel = 0`): no RSA key exchange, no RC4. Still needed for a
  production-capable server: real encrypted standard security (a
  proprietary-certificate signing key plus an RSA private-key decrypt path,
  neither implemented) and TLS/CredSSP server support (a certificate and a
  TLS server implementation); beyond the connection sequence, an actual
  server also needs to originate display updates and consume input, which
  `accept` does not attempt.

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
