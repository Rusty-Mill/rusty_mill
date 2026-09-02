# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]

Per-PR detail lives in [RELEASE_NOTES.md](./RELEASE_NOTES.md); this file summarises
by category for a reader who wants the shape of a release rather than its history.

### Added
- `service` and `dynamic` backends — a written-down service inventory, and a
  forward-proxy mode that takes its upstream from the request
- SNI-based certificate selection: one certificate per listener hostname on a port
- `protocol: TLS` passthrough via `tcpRoutes` — connections forwarded without being
  decrypted
- `mcpGuardrails` processors addressable by `backend:` or `service:` name
- The `gemini` provider, with streaming and tool calling
- `promptGuard`'s `openAIModeration` rule

### Changed
- `Endpoints` and `Retry` moved into `agentgateway-core`, shared rather than
  duplicated
- `gateway.rs` no longer has a catch-all backend arm; an unhandled kind is a
  compile error rather than a runtime 501

### Fixed
- Weighted traffic splitting across a `service` backend's instances — weights are
  divided, not repeated per instance
- Flaky cross-binary test port allocation
- Two clippy errors (`unnecessary_sort_by`, `useless_conversion`) that the
  development sandbox's older toolchain did not emit

### Infrastructure
- `rust-toolchain.toml` pins 1.97.0, so a local check and CI check the same thing.
  Added after a clean local run passed while CI failed on the same commit
- GitHub Actions CI running fmt, clippy under `-D warnings`, and the full test
  suite. Not yet a required status check, so it reports rather than gates

### Security
- A moderation key is never sent to a host it was not issued for; the gateway
  refuses to start rather than lending an Anthropic key to OpenAI's endpoint
- An open `dynamic` (forward-proxy) route is warned about at startup and reported
  by `--check`

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
