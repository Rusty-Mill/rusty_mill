# rusty_tonic

The protocol core of gRPC, with no I/O: no async, no HTTP/2, no code generation. A transport
hands in `(name, value)` header pairs and bytes and gets back typed values and header pairs to
send. Services, streaming and channels are built on top and live with the transport.

- **`Status` and `Code`:** the 17 codes, a message, typed details (`google.rpc.ErrorInfo` via
  `with_error_info` / `error_info`, from `rusty_prost`) and trailing metadata. `to_trailers()` makes
  `grpc-status`, `grpc-message` (percent-encoded outside printable ASCII), `grpc-status-details-bin`
  (unpadded base64 of a `google.rpc.Status`) and the metadata. `from_headers()` reads them back.
- **`MetadataMap`:** an ordered multimap. Keys are lowercase `[0-9a-z_.-]` and not protocol-reserved
  (`grpc-*`, `content-type`, `te`, ...); ASCII values are printable ASCII; `-bin` keys carry binary
  values as base64. Setting something invalid is an error and changes nothing. Reading received
  headers skips what cannot be metadata and returns how many it skipped.
- **Framing:** `encode_frame` / `encode_message` and `FrameDecoder` (5-byte prefix: compression flag,
  big-endian length). Bytes can arrive in any chunking. The receive limit (default 4 MiB) is checked
  when the prefix arrives, before any body is waited for or buffered. An error is final.
- **`grpc-timeout`:** `parse_timeout` / `format_timeout` (1 to 8 digits and `H M S m u n`; formatting
  uses the finest unit that fits and rounds up).
- **Heads:** `RequestHead` and `ResponseHead` to and from header lists; `ResponseStart` says whether a
  response is a normal start or "trailers-only". `code_from_http_status` is the spec's table.

**Limits.** Compression is not supported: a compressed message is `UNIMPLEMENTED`, and `identity` is
the only encoding. A malformed `grpc-status-details-bin` is dropped, not an error: the call keeps its
code and message. A message that fails to decode is `INTERNAL`, as in `tonic`.

**Hostile input.** Every parser takes untrusted bytes and returns an error or a skip; none panics.
`tonic`'s own `Status::from_header_map` panics on a malformed `grpc-status-details-bin` (an
`.expect(...)` on the base64), so one bad header can crash a `tonic` peer.

Tested against `tonic` as a dev-dependency oracle (its trailers read by us and ours by it), the
spec's examples, and property and corruption tests. The framing has no independent recorded
oracle yet: `tonic`'s framing is private, so it is checked against the spec's grammar here and
against live `tonic` traffic once the transport exists. The library builds on 1.75; the tests need
1.85, the floor of `tonic`.
