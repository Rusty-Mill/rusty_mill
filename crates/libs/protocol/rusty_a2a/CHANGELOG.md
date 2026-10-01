# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
### Security
- Webhook SSRF protection closes two bypasses (design review 3.6):
  - The address filter now blocks every non-global class. IPv6 unique-local (`fc00::/7`) and NAT64-embedded private IPv4 used to pass. Also added: deprecated site-local, shared/CGNAT, `0.0.0.0/8`, benchmarking, multicast and reserved ranges.
  - The DNS-pinned delivery client no longer follows redirects. A 3xx used to carry the payload to a host that was never checked. A redirecting webhook now counts as a failed delivery.

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
