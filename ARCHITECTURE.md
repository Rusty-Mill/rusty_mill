# Architecture

## Overview
`rusty_http` is one sans-IO HTTP/1.1 message layer (request/response head
parse + serialize, body framing, an incremental chunked-body state machine)
plus one `Url` type, replacing six hand-rolled implementations duplicated
today across `rusty_request` and `rusty_tail`. It sits "beside the PAL, not
on top" in the wider ecosystem's layer picture (`rustils/docs/architecture.md`),
the same shelf slot and seam discipline as `rusty_tls`. Not (yet) built:
see Boundaries below for what's planned vs. landed.

## Boundaries
Domain logic vs. I/O (ports-and-adapters): the sans-IO core never touches a
socket; adapters own all I/O and contain no parsing logic of their own.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Sans-IO message core — head parse/serialize, body-framing state machine | *(none — this crate's whole domain layer; planned, not yet built)* | byte-in/byte-out; head parsing must consume exactly the head so a caller mid-protocol-upgrade (Noise, DERP) can take the connection over byte-exact |
| Byte transport | `SyncStream` (any `std::io::Read + Write`, planned); `AsyncStream` (`rusty_tokio`'s `AsyncRead + AsyncWrite`, behind the `rusty-tokio` feature, planned) | thin — pumps bytes between the transport and the sans-IO core, owns no framing logic |

## Structure
Single crate, modular monolith — no forcing function (independent scaling,
a team/language boundary, hard fault isolation) applies to a parsing
library. Planned module boundaries: `url`, `header`, `method`, `status`,
the message/framing core, an optional `cookie` module behind the `cookies`
feature (client-only, RFC 6265), and the sync/async adapters described
above.

## Data flow
Planned: caller writes a request head + body through an adapter → adapter
relays bytes to/from the transport, unchanged → sans-IO core parses the
peer's response head, then hands body bytes back through the same framing
state machine (`Content-Length` / chunked / close-delimited) the adapter
started with. No diagram yet since no code has landed.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
HTTP/2, TLS (`rusty_tls`'s job — the two compose, neither imports the
other), compression, multipart (stays in `rusty_request` until a second
consumer wants it), routing frameworks.
