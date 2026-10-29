---
category: Changed
changelog: rp-router and rp-providers send on rusty_request instead of reqwest; hosts on tokio call `rusty_tokio::runtime::install_fallback()`.
---
## 2026-10-29 - rusty_provider's router and providers leave reqwest

- **Changed:** `rp-providers` (OpenAI-compatible, Anthropic, Gemini) and
  `rp-router`'s moderation, web search and webhook calls use
  `rusty_request` instead of `reqwest` (part of #687). Behaviour is
  unchanged: the per-provider timeout still bounds the whole exchange,
  streamed body included, the 16 MiB body cap and error classification
  stay, and errors never echo the request URL (Gemini's API key).
- **Changed:** a host that calls them from a tokio runtime must call
  `rusty_tokio::runtime::install_fallback()` once; `rp-server` already
  does (#1231), and its test harness now does too.
- **Known limitations:** `rp-router` stays on tokio for `tokio-postgres`
  and its spawned tasks, `rp-providers` keeps `eventsource-stream` and
  `futures-util` for SSE decoding, and `rp-server` keeps `reqwest` for
  its JWKS fetch.
