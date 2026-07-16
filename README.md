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
| BER (X.690) | `ber` | The definite-length TLV subset the MCS connection PDUs need. |
| PER (X.691) | `per` | The ALIGNED-PER subset the MCS domain PDUs need. |
| Byte cursors | `cursor` | Explicit big/little-endian, bounds-checked read/write. |

This is enough to build and parse the RDP connection handshake through MCS
channel setup: the X.224 negotiation, the BER `Connect-Initial`/`Response`
exchange, and the PER attach-user / channel-join / send-data domain PDUs. The
GCC (T.124) user data carried inside `Connect-Initial`/`Response` — the
client/server core, security, and network settings — is kept opaque for now
and is the next layer to decode. Security, capability exchange, and the
display/input channels build on top without disturbing what is here.

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
