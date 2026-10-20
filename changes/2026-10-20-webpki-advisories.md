---
category: Security
changelog: **Lockfile: webpki advisories.** `webpki-roots` and `webpki-root-certs` 1.0.10 and `rustls-webpki` 0.103.16 remove RUSTSEC-2026-0381, -0382 and -0383, which failed `cargo-deny` on every PR.
---
## 2026-10-20 - Lockfile: webpki advisories

- **Security:** `cargo update -p webpki-roots -p webpki-root-certs -p rustls-webpki@0.103.15`. RUSTSEC-2026-0381 and -0383 (1.0.9 bundles let paths through distrusted emSign intermediates) and RUSTSEC-2026-0382 (`rustls-webpki` 0.103.15) are fixed in 1.0.10, 1.0.10 and 0.103.16.
- `Cargo.lock` only: 6 lines, patch versions, no manifest change and no other crate moved. `cargo deny check advisories bans licenses sources` passes locally and `cargo check` builds.
