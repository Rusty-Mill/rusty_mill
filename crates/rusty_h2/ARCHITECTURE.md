# Architecture

## Overview
`rusty_h2` is a from-scratch HTTP/2 implementation, built directly from
RFC 9113 (HTTP/2) and RFC 7541 (HPACK). Today it provides the wire-format
layer only — frame codec, HPACK header compression, and the per-stream
state machine. There is no connection driver, async I/O integration, or
client/server API yet, so this document describes what actually exists,
not an aspirational end state.

## Boundaries
No I/O adapters exist yet: everything in the crate operates on in-memory
byte slices (`&[u8]` in, `Vec<u8>` out), so a literal ports-and-adapters
table would be empty scaffolding. The real boundary today is between the
protocol layers, each depending only on the ones below it:

| Layer | Depends on | Notes |
| ----- | ---------- | ----- |
| `stream` (RFC 9113 §5.1 state machine) | `error` | Pure state transitions driven by `Event`s; doesn't touch frame bytes — a connection driver will translate decoded frames into events. |
| `frame` (frame header + 10 frame types) | `error` | Encodes/decodes complete frames from byte slices; no knowledge of streams or HPACK. A `HEADERS`/`CONTINUATION` frame's header block is passed through as opaque bytes. |
| `hpack` (static table, Huffman, dynamic table, `Encoder`/`Decoder`) | `error` | Header compression only; turns a frame's `header_block_fragment` into/from a `Vec<HeaderField>`. |

When a connection driver is added, `frame` and `hpack` become the ports an
async I/O adapter (e.g. a `tokio::net::TcpStream` reader/writer) sits
behind — noting that now so this split doesn't need to be redesigned later.

## Structure
Modular monolith — a single crate, no workspace split. Ports-and-adapters
is the target pattern once I/O is introduced (see Boundaries above); today
the crate is pure protocol logic with nothing to adapt to, which reflects
where the project actually is rather than a gap to fill in.

## Data flow
Today: none — the crate exposes encode/decode functions, not a running
connection. Once a connection driver exists, the intended flow is:

```
raw bytes -> FrameHeader::decode + Frame::decode
          -> (HEADERS/CONTINUATION only) hpack::Decoder::decode
          -> stream::Stream::apply (state transitions)
          -> application-level headers/data delivered to the caller
```

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and
their tradeoffs.

## Non-goals (for now)
- No connection driver, async I/O integration, or client/server API (see
  README roadmap) — this is the next major body of work, not an oversight.
- No HTTP/1.1-to-HTTP/2 upgrade or ALPN negotiation handling.
- No flow-control window accounting yet: `WINDOW_UPDATE` frames parse and
  validate, but nothing enforces send/receive windows.
