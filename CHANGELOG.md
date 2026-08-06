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
### Changed
- `ARCHITECTURE.md` non-goals: "no dashboard"/"not multi-tenant SaaS"
  softened to "no UI" (ADR-0002).
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
