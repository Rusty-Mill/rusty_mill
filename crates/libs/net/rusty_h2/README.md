# rusty_h2

A from-scratch HTTP/2 implementation in Rust, built directly from
[RFC 9113](https://www.rfc-editor.org/rfc/rfc9113) (HTTP/2) and
[RFC 7541](https://www.rfc-editor.org/rfc/rfc7541) (HPACK).

## What's here

Previously, `src/connect/mod.rs` — the connection-level driver — was a
hollow shell: every non-trivial type (`FlowControl`, `Encoder`, `Decoder`,
`H2Error`, `Frame`, `StreamEntry`) was a bare placeholder duplicating real,
already-implemented types elsewhere in the crate, and its frame-dispatch
methods were no-ops (`Ok(vec![])`). Wiring `client`/`server`/`connect`
into `lib.rs` produced 23 compile errors. All of that is now real:

- **`frame`** — encode/decode for the 9-octet frame header and all ten
  standard frame types: `DATA`, `HEADERS`, `PRIORITY`, `RST_STREAM`,
  `SETTINGS`, `PUSH_PROMISE`, `PING`, `GOAWAY`, `WINDOW_UPDATE`, and
  `CONTINUATION`. Unknown frame types round-trip as opaque data rather
  than erroring, per RFC 9113 §4.1.
- **`hpack`** — a complete HPACK implementation: the 61-entry static
  table, a full Huffman codec (RFC 7541 Appendix B), integer/string
  primitives, an eviction-aware dynamic table, and an `Encoder`/`Decoder`
  pair supporting indexed fields, literal fields (with/without/never
  indexing), and dynamic table size updates.
- **`stream`** — the per-stream state machine from RFC 9113 §5.1
  (`idle` → `open`/`reserved` → `half-closed` → `closed`).
- **`error`** — shared error types distinguishing connection-level errors
  from stream-level errors, per RFC 9113 §5.4.
- **`connect`** — the real connection driver, built on the types above
  (not duplicates of them): `ServerSettings` negotiation (RFC 9113
  §6.5.2, with an ACK response), connection- and per-stream-level flow
  control (`WINDOW_UPDATE` application, `DATA` receive accounting),
  `PING`/ACK, `HEADERS` decode via the real HPACK `Decoder` driving the
  real per-stream state machine, `RST_STREAM`, `GOAWAY`, and a
  minimally-scoped `PUSH_PROMISE` (reserves the promised stream; doesn't
  deliver pushed response data — see Known gaps).
- **`client`/`server`** — thin wrappers over `connect::Connection`: a
  client `RequestBuilder`/`SendRequest`/`Connection::send_request` that
  encodes a request into HEADERS(+DATA) frames and applies them to the
  connection's own state machine, and a server `SendResponse`/`SendStream`
  that encode response HEADERS/DATA/RST_STREAM frames. Both return the
  frames they build; this crate still owns no I/O, so a caller's
  transport writes them to the wire.

`CONNECTION_PREFACE` (the 24-octet client preface from RFC 9113 §3.4) is
exported from the crate root.

## Known gaps

- **`PUSH_PROMISE` delivery**: the promised stream is reserved via the
  real state machine, but this driver never assembles/delivers the
  pushed response itself.
- **Mid-connection `SETTINGS_INITIAL_WINDOW_SIZE` changes** (RFC 9113
  §6.9.2) don't retroactively resize already-open streams' windows; only
  streams opened after the change pick up the new value.
- **`server::Connection::poll_accept`** always returns `Poll::Pending` —
  turning an inbound HEADERS frame into a queued `IncomingRequest` isn't
  implemented.
- **No async I/O driver** — callers own reading frames off, and writing
  the frames this crate builds to, the wire themselves.

## Testing

```
cargo test
cargo clippy --all-targets
```

The HPACK test suite includes the literal and Huffman-coded request
sequences straight from RFC 7541 Appendix C.3/C.4, decoded through a
single `Decoder` so the dynamic table evolves exactly as the RFC
describes.

## License

MIT — see `Cargo.toml`.
