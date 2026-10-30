---
category: Added
changelog: rusty_mux_transport, the pure core of rusty_mux's UDP transport: sealed datagrams carrying reliable ordered streams, with roaming.
---
## 2026-10-30 - rusty_mux UDP transport core

- **Added:** the crate `rusty_mux_transport` builds build step 1 of `docs/projects/mux/UDP-TRANSPORT.md` (Phase 3 step 4, C13). An `Association` is one key and one id with any number of streams, as a state machine with no clock, socket or thread.
  - **Datagrams** are `id | seq | ciphertext | tag`, sealed with ChaCha20-Poly1305 from `rusty_aead`, and at most 1200 bytes. Each side checks the direction bit, the id, the tag and a 1024-entry replay window, in that order, before any state changes.
  - **Roaming:** a server sends to the source of the authenticated datagram with the highest sequence number.
  - **Streams** are reliable and ordered. Each has a 1 MiB receive window advertised in its acks, and writes stop at 1 MiB unacknowledged. Losses are found from SACK ranges, and a fast retransmit fires once three datagrams' worth beyond a hole is acknowledged.
  - **Pacing:** RFC 6298 retransmission timers stay between 100 ms and 2 s, with a NewReno-style window.
  - **Other chunks:** a probe, answered at once even before any stream opens; keepalives after 3 s of silence; stream closes confirmed by the peer, and told again if the peer later uses a closed stream; and an association end.
  - **Bounds:** a stream holds at most 1 MiB received in at most 1906 non-overlapping pieces; at most 64 skipped stream numbers wait to open; closes are packed within the 1200-byte datagram.
- **Changed:** `rusty_aead` leaves the workspace's cargo-shear ignore list, since a member now uses it.
- **Known limitations:** nothing uses the crate yet. The server and the client adopt it in the next build steps. The crate declares `rust-version` 1.98.1, which is what `rusty_aead` needs.
