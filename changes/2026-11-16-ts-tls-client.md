---
category: Added
changelog: ts-derp, ts-control, ts-engine and ts-net can reach an https:// control or DERP server, trusting the system anchors or a CA file; ts-daemon gets --tls-ca.
---
## 2026-11-16 - TLS-capable ts2021 and DERP clients (mesh hub, follow-up to H5)

- **Added:** `https://` control and DERP URLs. A new small crate `ts-dial` opens the outbound stream (TCP, then TLS for `https://`) behind one boxed type; `ts-derp` and `ts-control` keep their protocol code over plain `AsyncRead + AsyncWrite`. Trust is a `rusty_tls::TrustPolicy`, re-exported as `Trust`: `System` (default), or `PinnedAnchors` for a private CA or the hub's own certificate.
- **Public API (additive):** `DerpClient::connect_with(url, key, &Trust)`, `ControlClient::connect_with(url, key, hostinfo, Trust)`, `ControlUrl::with_trust` and `ControlUrl::tls`, `EngineConfig::trust` and `NodeConfig::trust` (new required fields; every caller in the workspace updated), `ts_dial::{connect, scheme, server_name, trust_from_pem, pem}`. `ControlUrl::parse` now accepts `https://` (port 443); `dial` returns `Conn<Replay<BoxIo>>`.
- **CLI:** `ts-daemon --tls-ca FILE` trusts only the certificates in that PEM file for `https://` URLs; a file with no certificate stops the daemon with exit code 2 before it dials.
- **Changed:** `ts-hub` reads PEM through `ts_dial::pem` (the same reader, no longer a private copy). `ts-derp`'s declared `rust-version` rises from 1.85 to 1.88, which its new `ts-dial` dependency (through `rusty_tls`) needs.
- **Dependencies:** new workspace crate `ts-dial` (no new external crate); `rcgen 0.13` becomes a dev-dependency of `ts-net` for test certificates (already used by `ts-hub`).
- **Limits:** the TLS handshake runs on the first read or write, so a refused certificate surfaces there, inside the caller's own timeout. The data path pays one `dyn` call per read and write.
- **Not in this PR:** interop with stock `tailscaled` (not installed in the build environment); ACME.
- **Tested:** two `ts-net` nodes join a hub behind verified TLS and move 1 MiB byte for byte; a node pinning another certificate cannot join; `https://` URL parsing (default port 443); PEM reader (blocks, malformed input without echoing content); `--tls-ca` with no certificate exits 2; the existing 4 MiB plain transfer still passes.
