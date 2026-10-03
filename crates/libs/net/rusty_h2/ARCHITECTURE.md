# Architecture

## Overview
`rusty_h2` is a from-scratch HTTP/2 implementation, built directly from
RFC 9113 (HTTP/2) and RFC 7541 (HPACK). It provides a frame codec, HPACK
header compression, a per-stream state machine, a connection driver, and
thin client/server wrappers. The crate deliberately owns no I/O: callers
feed decoded frames into the connection and write the frames it returns.

## Boundaries
No I/O adapters exist: everything in the crate operates on in-memory frame
values and byte buffers. The client and server APIs build frames but do not
read from or write to sockets. The protocol layers depend only on the ones
below them:

| Layer | Depends on | Notes |
| ----- | ---------- | ----- |
| `stream` (RFC 9113 §5.1 state machine) | `error` | Pure state transitions driven by `Event`s; the connection driver translates decoded frames into events. |
| `frame` (frame header + 10 frame types) | `error` | Encodes/decodes complete frames from byte slices; no knowledge of streams or HPACK. A `HEADERS`/`CONTINUATION` frame's header block is passed through as opaque bytes. |
| `hpack` (static table, Huffman, dynamic table, `Encoder`/`Decoder`) | `error` | Header compression only; turns a frame's `header_block_fragment` into/from a `Vec<HeaderField>`. |
| `connect` (connection driver) | `error`, `frame`, `hpack`, `stream` | Owns negotiated settings, stream state, flow control, header-block assembly, frame dispatch, and immediate PING acknowledgements. |
| `client` / `server` | `connect`, `frame`, `hpack` | Thin request/response frame-building APIs over the connection driver; transport I/O remains the caller's responsibility. |

An eventual sync or async I/O adapter can sit outside these layers without
moving socket ownership into the protocol core.

## Structure
Modular monolith — a single crate, no workspace split. The in-memory
protocol and connection layers form the inside of a ports-and-adapters
boundary; a future I/O adapter remains outside that boundary.

## Data flow
Callers decode raw bytes into `Frame` values and pass each incoming frame to
`connect::Connection::apply_frame` (or its `apply` alias). The connection
driver dispatches by frame type. For `HEADERS` and `PUSH_PROMISE`, it assembles
and validates any required `CONTINUATION` sequence, waiting for `END_HEADERS`
before HPACK decoding and the relevant stream-state transitions. `SETTINGS`,
`PING`, and other control frames use their own handlers. Any response/control
frames are returned for caller-owned transport; application-level request or
push delivery is not implemented.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and
their tradeoffs.

## Non-goals (for now)
- No sync or async I/O integration; callers own transport reads and writes.
- No byte-level connection-preface validation. The crate exports the client
  `CONNECTION_PREFACE` bytes, but does not parse either peer's preface.
- No scheduled keepalive policy or outstanding-PING timeout tracking. The
  connection driver only acknowledges received non-ACK PING frames.
- No HTTP/1.1-to-HTTP/2 upgrade or ALPN negotiation handling.
- Flow control is enforced by `connect::Connection` alone: receive windows
  refuse overruns and are replenished by `release_capacity`; outgoing
  frames go through `send_frame`, never the receive path.
