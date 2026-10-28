---
category: Fixed
changelog: remind-me capture hook sends its own User-Agent, so a Cloudflare-fronted endpoint no longer refuses it with HTTP 403 (ADR-0029).
---
## 2026-10-28 - remind-me: capture hook passes Cloudflare

- **Fixed:** Cloudflare's browser integrity check refuses urllib's default `Python-urllib` agent (error 1010), so every cloud capture to a Cloudflare-fronted endpoint failed with `http 403`. The hook now sends `User-Agent: remind-me-capture/1`.
- **Known limitations:** environments pin the hook by commit, so this reaches one only once its setup-script `REF` is bumped.
