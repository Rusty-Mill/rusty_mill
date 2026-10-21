---
category: Added
changelog: "`rusty_h2`: `ClientHandle::ready` and `Acceptor::ready`, which wait for the peer's first SETTINGS (on a server, after the client's preface), so a caller can bound the handshake and tell a peer that only answers PINGs from one that is ready."
---
## 2026-10-21 - rusty_h2: readiness is the peer's first SETTINGS

- **Why:** found in review of `rusty_hyper` (#826). `connect` used a PING round trip as proof that the peer speaks HTTP/2, but `rusty_h2` acknowledges a PING before it validates anything else, and `send_request` separately waits for the server's SETTINGS. A peer that answers PINGs and never sends SETTINGS made `connect` succeed and the first request hang outside `connect_timeout`. On the server side, nothing could tell a client that stalled part-way through its preface from one that was ready, so it could hold a connection slot indefinitely.
- **Added:** `ClientHandle::ready()` and `Acceptor::ready()` (feature `rusty-tokio`). Each resolves when the peer's first SETTINGS have been applied (the same condition `send_request` already waits for), and fails if the connection closes or the peer announces GOAWAY first. On a server the preface precedes SETTINGS, so it covers both.
- **Tests (written first; all 6 failed on `todo!()`):** in `tests/driver.rs`, each waiting on its own task so the wake-up is what finishes it: a client is ready only after the server's SETTINGS; a peer that acknowledges a PING and sends no SETTINGS is not ready (the case that motivated this); a closed connection is never ready, client and server; a server stays not ready through one byte of the preface and through the whole preface, and is ready on SETTINGS; both sides are ready after a real handshake. Mutation-checked: skipping the SETTINGS check, the closed check, or the waker registration each fails tests.
- **Not changed:** no existing behavior.
