# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Initial Cargo workspace (`skillopt-core`, `skillopt-model`, `skillopt-envs`,
  `skillopt-cli`): a hand-rolled reimplementation of SkillOpt's rollout ->
  reflect -> optimize -> validation-gate skill-training loop.
- `ChatBackend` adapters for Anthropic and any OpenAI-compatible endpoint,
  plus a network-free Mock backend.
- `synthetic_arithmetic` `Environment` with configurable multi-step chaining
  and distractor density for exercising the loop offline or against a real
  model at adjustable difficulty.
- `skillopt train`/`eval` CLI, example configs (`example.yaml` for an offline
  mock dry run; `smoke_claude.yaml`, `smoke_claude_hard.yaml`, and
  `full_claude.yaml` for real Anthropic-backed runs).
- Repo governance scaffolding (this changelog, RELEASE_NOTES, CONTRIBUTING,
  SECURITY, CODE_OF_CONDUCT, ARCHITECTURE, ADR log, PR/issue templates, CI).
- `configs/ollama_example.yaml` — the `openai_compatible` provider now works
  against Ollama (and other no-auth local OpenAI-compatible servers) with no
  API key env var needed at all.
- `Provider::AzureOpenAi` + `AzureOpenAiBackend` (`api-key` header auth,
  resource-endpoint + deployment-name URL shape, optional `api_version`),
  and `configs/azure_openai_example.yaml`.
- `configs/qwen_example.yaml` — Qwen via DashScope's OpenAI-compatible mode,
  using the existing `openai_compatible` provider with no code changes.

### Changed
- `openai_compatible`'s API key is now optional: no `Authorization` header
  is sent when none is configured, instead of erroring.

### Fixed
- reqwest now uses the OS native root store instead of a fixed bundled trust
  store, so outbound HTTPS works through environments with a TLS-terminating
  egress proxy.
- Workspace formatted to rustfmt defaults so the new CI workflow starts green.
- `synthetic_arithmetic`'s distractor sentences could name the protagonist
  themself, producing self-contradictory problem text; distractor name
  selection now excludes the protagonist.
- `Provider`'s YAML representation for `openai_compatible` had silently
  derived as `open_ai_compatible`, contradicting every doc/example config in
  the repo; never previously exercised by a real run.

### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
