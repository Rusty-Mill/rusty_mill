# Architecture

## Overview
`rusty_http` is one sans-IO HTTP/1.1 message layer (request/response head
parse + serialize, body framing, an incremental chunked-body state machine)
plus one `Url` type, replacing six hand-rolled implementations duplicated
today across `rusty_request` and `rusty_tail`. It sits "beside the PAL, not
on top" in the wider ecosystem's layer picture (`rustils/docs/architecture.md`),
the same shelf slot and seam discipline as `rusty_tls`. `Url` and the
sans-IO message core are built and tested; adapters are not — see
Boundaries below for what's planned vs. landed.

## Boundaries
Domain logic vs. I/O (ports-and-adapters): the sans-IO core never touches a
socket; adapters own all I/O and contain no parsing logic of their own.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Sans-IO message core — `head::parse_request_head`/`parse_response_head` (byte-exact head parse + serialize), `body::request_framing`/`response_framing` + `body::ChunkedDecoder` (body framing) | *(none — this crate's whole domain layer)* | **built.** Byte-in/byte-out; head parsing consumes exactly the head (pinned by tests using donor 4's exact scenario — trailing non-HTTP bytes after the blank line are provably untouched), so a caller mid-protocol-upgrade (Noise, DERP) can take the connection over byte-exact |
| Byte transport | `SyncStream` (any `std::io::Read + Write`, planned); `AsyncStream` (`rusty_tokio`'s `AsyncRead + AsyncWrite`, behind the `rusty-tokio` feature, planned) | **not yet built** (next step) — thin, pumps bytes between the transport and the sans-IO core, owns no framing logic |

## Structure
Single crate, modular monolith — no forcing function (independent scaling,
a team/language boundary, hard fault isolation) applies to a parsing
library. Landed modules: `url` (`Url`), `header` (`HeaderMap`), `method`
(`Method`), `status` (`StatusCode`), `version` (`Version`), `head`
(request/response head parse + serialize), `body` (framing + the
incremental `ChunkedDecoder`), `util` (private line-scanning helper),
`error`. Not yet built: an optional `cookie` module behind the `cookies`
feature (client-only, RFC 6265), and the sync/async adapters described
above.

## Data flow
Planned (no adapter exists yet to drive it): caller writes a request head +
body through an adapter → adapter relays bytes to/from the transport,
unchanged → sans-IO core parses the peer's response head, then hands body
bytes back through the same framing state machine (`Content-Length` /
chunked / close-delimited) the adapter started with.

## Security-relevant bounds, decided during the core's build
Both head parsing and chunked-body-framing-line parsing take an explicit
`max_len` bound (`head::DEFAULT_MAX_HEAD_LEN`/`body::DEFAULT_MAX_LINE_LEN`,
both 8 KiB) and return an error rather than an endless "need more data" once
a buffer exceeds it without completing. This wasn't in the original mission
handoff's sequencing — added because this core parses untrusted,
server-bound requests (see donor 6, the LocalAPI server), and an unbounded
head/line is a memory-exhaustion vector against exactly that consumer. A
caller parsing only trusted peers (e.g. a client reading its own server's
response) can pass a larger bound.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
HTTP/2, TLS (`rusty_tls`'s job — the two compose, neither imports the
other), compression, multipart (stays in `rusty_request` until a second
consumer wants it), routing frameworks.
