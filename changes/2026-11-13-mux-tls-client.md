---
category: Added
changelog: rusty-mux-ctl --host can reach a TLS host pinned to its certificate's fingerprint, logged in with a token file (#506, client side).
---
## 2026-11-13 - rusty_mux TLS client side

- **Added:** `rusty-mux-ctl hosts add NAME --tls HOST:PORT --pin HEX` and `--pin-first-use` (pins the certificate the server presents now and prints its fingerprint). A TLS address is saved as `{"tls": "host:port", "pin": "<64 hex>"}`; the pin is not secret, but TLS addresses are not synced by `hosts pull|push` yet.
- **Added:** `--host NAME` reaches a TLS host through a local relay socket (`rusty_mux_client::tls::Relay`, Unix only). The server is trusted only if its certificate matches the pin (`rusty_tls` `PinnedFingerprints`); a changed certificate is refused with both fingerprints and nothing re-pins by itself. The host's token is read from `tokens/NAME` next to the hosts file (a regular file only its owner can access, 64 hex digits, as `rusty-mux-ctl token create` prints) and is never in `hosts.json`.
- **Known limitations:** needs the server side (#1671). Unix only. The terminal client has no `--host` for TLS yet, and the GUI is untouched. No interactive trust prompt: use `--pin` or `--pin-first-use`. New workspace edges only (`rusty_tls`, `rusty_tokio` into `rusty_mux_client`); no wire change and no third-party dependency.
