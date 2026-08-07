# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
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
