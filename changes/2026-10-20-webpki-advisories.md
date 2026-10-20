---
category: Security
changelog: "Updated `rustls-webpki` to 0.103.16 and `webpki-roots` / `webpki-root-certs` to 1.0.10, which removes the distrusted emSign root (RUSTSEC-2026-0381, -0382, -0383) and clears the `cargo-deny` advisories gate."
---
## 2026-10-20 - webpki advisories

- **Why:** `cargo-deny (advisories, licenses, sources)` fails on `main` for three advisories in locked TLS dependencies: `webpki-roots` 1.0.9 (RUSTSEC-2026-0381), `rustls-webpki` 0.103.15 (RUSTSEC-2026-0382) and `webpki-root-certs` 1.0.9 (RUSTSEC-2026-0383). The bundled roots include an emSign root whose intermediates are distrusted (their private keys are public), so certificate paths through them could be accepted. Every open PR inherits the red gate.
- **Changed:** `Cargo.lock` only: `rustls-webpki` 0.103.15 → 0.103.16, `webpki-root-certs` 1.0.9 → 1.0.10, `webpki-roots` 1.0.9 → 1.0.10 (`cargo update -p` for those three). No manifest, `deny.toml` or policy change, no ignores added.
- **Checked:** `cargo deny check advisories` is `ok`; `cargo check --locked` of `rusty_tls`, `rusty_request` and `agentgateway-tls`, which depend on them.
- **Not checked:** the full workspace test suite against the new versions. The updates are patch releases of leaf TLS crates.
