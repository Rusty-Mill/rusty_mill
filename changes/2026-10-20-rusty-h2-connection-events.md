---
category: Changed
changelog: "`rusty_h2`: the connection now delivers decoded headers, body bytes, trailers, resets and GOAWAY to the application and can send headers, data and trailers; `poll_accept` works; flow-control windows follow the settings each side advertised."
---
## 2026-10-20 - rusty_h2: connection events, trailers and windows

- **Why:** phase 4a of the first-party `prost`/`tonic` plan (card t45). `rusty_h2` dropped every decoded header block and DATA payload, so nothing above it could be built; gRPC needs trailers. Still sans-IO: the async driver is the next PR.
- **Added:** `connect::Inbound` (`Headers`, `Data`, `Trailers`, `Reset`, `GoAway`) drained with `Connection::take_inbound`; `open_stream`, `send_headers`, `send_data` (split at the peer's maximum frame size, refused whole if it does not fit the windows) and `send_trailers`, all encoding with the connection's single HPACK state. `server::Connection::poll_accept` yields requests; `take_inbound` returns the rest.
- **Changed:** header blocks are validated (lowercase names, known pseudo-headers first, none in trailers; a second block must end the stream), as stream errors after HPACK has consumed them. A change to `SETTINGS_INITIAL_WINDOW_SIZE` now moves open streams' send windows (and may leave one negative), and a value over 2^31-1 is a connection error. The receive window of a new stream is now the size this endpoint advertised; it was the peer's.
- **Tests:** focused coverage in `tests/events.rs`, including transactional HPACK rejection, invalid sending states, and preservation of request HEADERS `END_STREAM`.
- **Found, not fixed:** `server::SendResponse::send_response` encodes with a fresh HPACK encoder per response, which desyncs a peer's decoder on the second response; use `Connection::send_headers`. 1xx informational responses are delivered as the first block. A header block larger than the peer's maximum frame size is refused (no CONTINUATION on send). The HPACK encoder's table size follows the local setting, not the peer's.
