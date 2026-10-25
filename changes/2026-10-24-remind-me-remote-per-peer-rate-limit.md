---
category: Fixed
changelog: remind_me_remote serves with connect-info, so the rate limiter buckets anonymous and OAuth callers per peer address instead of in one shared `ip:unknown` bucket (#920).
---
## 2026-10-24 - Per-peer rate-limit buckets for the remote connector

- **Fixed:** `remind_me_remote` now serves its router with `into_make_service_with_connect_info::<SocketAddr>()`
  through a new `remind_me_remote::serve`. Without it the secret gate never saw a peer address, so every
  caller not presenting the connector secret, OAuth users included, shared one `ip:unknown` bucket and a
  single noisy client could exhaust it for everyone (#920).
- **Known limitations:** behind a tunnel or reverse proxy the peer is the proxy itself, so callers arriving
  through it still share that proxy's bucket. Forwarded-for headers are not trusted.
