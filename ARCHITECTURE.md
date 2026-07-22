# Architecture

## Overview
`rusty_http` is one sans-IO HTTP/1.1 message layer (request/response head
parse + serialize, body framing, an incremental chunked-body state machine)
plus one `Url` type, replacing six hand-rolled implementations duplicated
today across `rusty_request` and `rusty_tail`. It sits "beside the PAL, not
on top" in the wider ecosystem's layer picture (`rustils/docs/architecture.md`),
the same shelf slot and seam discipline as `rusty_tls`. `Url`, the sans-IO
message core, all three transport adapters, and the `cookies` feature are
built and tested — see Boundaries below. Not yet built:
`rusty_tail`'s migration itself.

## Boundaries
Domain logic vs. I/O (ports-and-adapters): the sans-IO core never touches a
socket; adapters own all I/O and contain no parsing logic of their own.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Sans-IO message core — `head::parse_request_head`/`parse_response_head` (byte-exact head parse + serialize), `body::request_framing`/`response_framing` + `body::ChunkedDecoder` (body framing) | *(none — this crate's whole domain layer)* | **built.** Byte-in/byte-out; head parsing consumes exactly the head (pinned by tests using donor 4's exact scenario — trailing non-HTTP bytes after the blank line are provably untouched), so a caller mid-protocol-upgrade (Noise, DERP) can take the connection over byte-exact |
| Byte transport | `sync::SyncTransport<T: std::io::Read + Write>`; `async_tokio::AsyncTransport<T: rusty_tokio::io::AsyncRead + AsyncWrite>` (behind `rusty-tokio`); `tokio_native::AsyncTransport<T: tokio::io::AsyncRead + AsyncWrite>` (behind `tokio`, real crates.io tokio) | **built.** Thin — pumps bytes between the transport and the sans-IO core, owns no framing logic of its own; each has its own error type (`TransportError`) wrapping I/O failures alongside the core's own `Error`. All three support both eager (`read_body`, whole body in memory) and incremental (`into_body_reader`/`BodyReader::next_chunk`, one chunk at a time) reads |

## Structure
Single crate, modular monolith — no forcing function (independent scaling,
a team/language boundary, hard fault isolation) applies to a parsing
library. Landed modules: `url` (`Url`), `header` (`HeaderMap`), `method`
(`Method`), `status` (`StatusCode`), `version` (`Version`), `head`
(request/response head parse + serialize), `body` (framing + the
incremental `ChunkedDecoder`), `sync` (`SyncTransport`), `async_tokio`
(`AsyncTransport` over `rusty_tokio`, behind the `rusty-tokio` feature),
`tokio_native` (`AsyncTransport` over real tokio, behind the `tokio`
feature), `transport` (shared adapter error type), `cookie` (`CookieJar`,
behind the `cookies` feature, client-only, RFC 6265), `util` (private
line-scanning helper), `error`.

`sync`, `async_tokio`, and `tokio_native` are deliberately three parallel,
near-identical modules rather than one written generic over blocking-ness
(or generic over which async runtime) — Rust has no good code-sharing
story for either axis without a proc-macro or maybe-async/maybe-runtime
crate, and `rusty_tls` already established the sync/async half of this as
the ecosystem's answer (`client.rs`/`async_client.rs`, same shape); adding
`tokio_native` alongside `async_tokio` extends the same pattern to a
second axis (which runtime) rather than inventing a new one.

## Data flow
Caller (via one of the three transports) writes a request head + body →
adapter relays bytes to/from the transport, unchanged → sans-IO core
parses the peer's response head, then hands body bytes back either
eagerly (`read_body`) or incrementally (`into_body_reader`, one chunk at
a time via `BodyReader::next_chunk`) through the same framing state
machine (`Content-Length` / chunked / close-delimited) the adapter
started with. Exercised end-to-end in all three adapters' test suites (an
in-memory `Read + Write` loopback for `sync`, a `duplex` pair for each of
`async_tokio`/`tokio_native`, over their respective runtime) — not yet
against a real socket or a real peer.

The incremental path (`into_body_reader`) wasn't in the original mission
handoff's step 3 either — added while planning the `rusty_request`
migration (step 4): donor 1's `http1.rs` had both an eager and a
streaming response-reading path (`send_request`/`send_request_streaming`),
and `rusty_request`'s own `send_streaming`/`StreamingResponse` depends on
the streaming half. Without it, migrating `rusty_request` could only ever
replace the eager path, leaving a second, still-duplicated body reader
behind -- the same "found a real gap, closed it before migrating" pattern
that also produced the `cookies` feature and the head/line size bounds.

## A gap found while building the adapters, and how it was resolved
Step 3's sequencing assigned `rusty_tail`'s four donor sites to the sync
adapter ("sync over `std::io`"). Source review showed this was wrong: all
four (`ts-control/controlhttp.rs`, `ts-derp/client.rs`,
`ts-cli/localapi.rs`, `ts-localapi/lib.rs`) are already async, built on
real crates.io `tokio` — not `std::io`, and not `rusty_tokio` either (see
`rusty_tail`'s workspace `Cargo.toml`: `tokio = { version = "1", ... }`).
Neither adapter built in step 3 fit those call sites as-is:
`sync::SyncTransport` would mean blocking a tokio worker thread (or
routing through `spawn_blocking`), and `async_tokio::AsyncTransport`
requires `rusty_tokio`'s own `AsyncRead`/`AsyncWrite` traits, which real
`tokio`'s `TcpStream`/`UnixStream` don't implement.

**Resolved**: a third adapter, `tokio_native::AsyncTransport` (behind the
`tokio` feature), driving the sans-IO core over real tokio's own
`AsyncRead`/`AsyncWrite` -- chosen over the two alternatives considered
(a `spawn_blocking` bridge onto `SyncTransport`, or a `rusty_tail` runtime
migration onto `rusty_tokio`) because real tokio's I/O traits are already
close enough in shape to `rusty_tokio`'s (unsurprising -- that's what
`rusty_tokio` was modeled on) that the adapter is a small, mechanical
addition, whereas `spawn_blocking` would need a full actor-task-plus-
channel design to own a blocking transport for a connection's whole
lifetime (real per-call overhead, and a shape mismatch with how the rest
of this crate's adapters are used directly, not through a channel), and a
`rusty_tail` runtime migration is a far larger, unrelated undertaking this
crate's mission was never meant to force. `rusty_tail`'s own migration
(replacing its four hand-rolled/`hyper`-based call sites with this
adapter) is the next step, not done in this repo.

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
