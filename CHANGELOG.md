# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `server.cors_allowed_origins` — restricts CORS to an explicit browser-
  origin allowlist. Unset preserves the existing any-origin behavior.
- `[webhook]` HMAC-SHA256 signing (`signing_secret_env`) and
  retry-with-backoff (`retry_backoff_secs`/`retry_backoff_max_secs`/
  `max_retries`) — a `5xx`/network-error delivery now retries instead of
  failing after one attempt; a `4xx` is still treated as permanent.
- `[[clients]].budget_warning_threshold` — a new `budget_warning`
  `[webhook]` event fires once a client's spend crosses this fraction of
  `budget_usd`, ahead of the hard `budget_exceeded` cutoff. Config-only
  for now, not yet settable via the admin API.
- Reasoning replay for tool-continuation turns — some OpenAI-compatible
  reasoning models (DeepSeek-reasoner, Kimi-K-series, QwQ, GLM-thinking)
  reject a tool-answering turn missing the `reasoning_content` behind the
  tool call. A non-streaming response's reasoning is now cached in memory
  by `tool_calls[].id` and transparently re-injected into the matching
  assistant message on the next request, even when the calling client
  stripped it (most do).
- `docs/PROVIDERS.md` + curated commented-out provider presets in
  `config.example.toml` (~20 more OpenAI-wire-compatible backends).
- `[[free_tiers]]` config + `GET /v1/free-tiers` — operator-declared,
  self-tracked free-token budget reporting per "provider/model".
- Three new `provider.sort` strategies: `"quality"`, `"random"`,
  `"free_tier_remaining"`.
- `transforms: ["rtk"]` — built-in tool-output compression (git/test/
  build/package/generic categories), composable with `"middle-out"`.
- `rp-cli` — new 5th workspace crate, a read-only operator CLI (`config
  check`/`providers list`/`keys check`).
- `[cache].mode = "semantic"` — embedding-cosine-similarity response
  caching, opt-in alongside the existing exact-match mode.
- `[jwt]` — JWT/OIDC bearer-token authentication (HS256 shared-secret or
  JWKS/RS256), additive alongside `server.api_key_env`/`[[clients]]`.
  Fails closed on any verification failure.
- `rp-cli setup` (`list`/`show`/`apply`) — rewrites a known third-party CLI
  tool's own config file (opencode, Crush) to point its endpoint at
  rusty_provider. Data-driven target list (`crates/cli/cli_targets.toml`,
  extensible via `--targets`), dry-run by default, `--yes`-gated writes
  with an automatic backup, never writes a literal API key (an env-var
  reference naming `--api-key-env` when the target format supports one).
  Static file rewriting only — no proxy, no traffic interception (ADR-0004).
- `strategy = "fusion"` on a `[[routes]]` alias — dispatches the alias's
  `chain` (the "panel") in parallel instead of sequentially, then
  synthesizes one final answer via a designated `judge` model from
  whichever candidates responded within `fusion_timeout_secs` (each
  independently timed out, so the total wait doesn't scale with panel
  size). Panel answers reach the judge under an anonymized label, not by
  provider/model. A tool-calling or streaming request bypasses fusion
  entirely and falls back to ordinary sequential-chain dispatch, as does a
  fusion alias with no `judge` configured (a startup warning, not a hard
  failure). Usage/cost accounting covers every contributing panel member
  plus the judge, not just the judge's own call.
- `provider.max_request_price_usd` + `provider.budget_fallback` — caps a
  single request's estimated cost (`max_tokens * completion_per_million`
  per candidate). `budget_fallback: "strict"` (default `"cheapest"`)
  narrows the chain to only candidates under the cap, `402`-ing if none
  fit; `"cheapest"` always serves the request, routing to the cheapest
  fitting candidate or, failing that, the overall cheapest one anyway.
### Changed
- `ARCHITECTURE.md` non-goals: "no dashboard"/"not multi-tenant SaaS"
  softened to "no UI" (ADR-0002); "no MITM-based third-party CLI config
  injection" narrowed to "no traffic interception" now that `rp-cli setup`
  covers the static config-file-rewriting case (ADR-0004).
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
