# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- **Hand-rolled TLS engine, stage 1: the TLS 1.3 record layer** (rusty_tls#25).
  A new `handrolled::record` module implementing RFC 8446 §5 — AEAD
  protection, framing, inner content types, padding, and the §5.3 nonce
  construction — over AES-128-GCM, AES-256-GCM, and ChaCha20-Poly1305.
  Nothing in the crate's public API routes through it; `rustls` remains the
  engine behind every exported type.
- `handrolled-engine` cargo feature, **off by default and never to become a
  default**, which must be combined with `--cfg rusty_tls_handrolled` for the
  module to compile at all. Two gates rather than one because cargo features
  are unified across a dependency graph and a `--cfg` flag is not — see
  ADR-0002. Enabling the feature without the cfg compiles a documented stub
  module explaining how to enable it, rather than silently doing nothing.
- `ring` as an optional direct dependency, enabled by `handrolled-engine`.
  Already present transitively as rustls' crypto provider, so this adds a
  dependency edge rather than new code to any build.
- ADR-0002, recording the never-default guarantee as a binding decision,
  the staging order for the remaining work, and the bar each stage must
  clear before it lands.
- A second CI job that builds and tests the hand-rolled engine with the cfg
  set, including a check that the gated tests actually ran — a typo in the
  cfg name would otherwise compile the suites down to zero tests and still
  report success.
- **Hand-rolled engine, stage 2a: DER decoding and X.509 certificate
  parsing** (rusty_tls#25). `handrolled::der` is a strict DER reader that
  refuses every non-canonical encoding DER forbids; `handrolled::x509` parses
  certificates on top of it, keeping `tbsCertificate`, `issuer`, and
  `subject` as borrows of the original bytes rather than re-encoding them.
  Understood extensions are `basicConstraints`, `keyUsage`,
  `extendedKeyUsage`, and `subjectAltName`; critical extensions that are
  *not* understood are collected and reported so a validator can comply with
  RFC 5280 §6.1.3(f).

  **This validates nothing** — no signature check, no clock, no chain, no
  name matching. That is stage 2b, which does not exist yet. ADR-0002's
  staging table is updated to record the split and why.
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
