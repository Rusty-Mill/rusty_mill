# ADR-0001: HTTP API on `rusty_http` and std threads

Status: Accepted
Date: 2026-09-29

## Context

`rusty_tick` needs a JSON HTTP API. Other apps in the monorepo use axum/hyper
on tokio; `rusty_tailscale`'s LocalAPI serves HTTP on `rusty_http` (its own
sans-IO HTTP/1.1 layer). ADR-0002 (dependency sovereignty) prefers the
monorepo's own crates where one exists.

## Decision

1. **Framing, JSON and query decoding come from `rusty_http`, `rusty_json` and
   `rusty_url`**, not hyper, serde_json or `url`.
2. **The router is sans-IO** (`api::Api::handle`: request in, response out) and
   the socket adapter (`server`) only moves bytes, so every route is tested
   without a network and the transport can change without touching routes.
3. **Blocking `std::net` with one thread per connection, no async runtime.**
   The workload is a few concurrent clients and one store behind one `Mutex`;
   there is no I/O concurrency for a runtime to exploit. Connection count,
   head size (16 KiB), body size (1 MiB) and idle time (30 s) are bounded.
4. **Bearer-token auth from `RUSTY_TICK_TOKEN`** (at least 16 characters,
   compared in constant time). Plain HTTP, so the binary refuses a
   non-loopback address unless `--allow-remote` says TLS is terminated in
   front of it.

## Consequences

- Chunked *request* bodies are refused (411); clients send `Content-Length`.
- No TLS, no HTTP/2, no WebSocket push in this layer. Sync clients will need
  polling or a later transport.
- A slow handler holds the store lock and blocks other requests. Acceptable at
  this scale; revisit with a read/write split if it is not.
- Moving to an async runtime later means replacing `server.rs` only.
