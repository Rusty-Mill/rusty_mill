# TLS client side (card #506): what is built

The client half of [TLS-LISTENER.md](TLS-LISTENER.md), after the server side (PR #1671) and the `rusty_tls` fingerprint pin (`TrustPolicy::PinnedFingerprints`, #1673). Nano gave the go in the project thread (2026-11-13 22:23Z).

- **Address:** `{"tls": "HOST:PORT", "pin": "<64 lowercase hex>"}` in `hosts.json` (`rusty_mux_script::hosts::Address::Tls`). The pin is a public fingerprint, so the file stays free of secrets. `hosts pull|push` still carries only ssh addresses; a pin that arrived by sync would be trusted as it stands, which needs its own decision.
- **Pinning:** `hosts add NAME --tls ADDR --pin HEX`, or `--pin-first-use`, which takes the fingerprint of the certificate the server presents now (handshake only, no token sent) and prints it to compare. There is no interactive prompt. A certificate that later differs is a hard failure that prints both fingerprints; nothing re-pins by itself, and the next address is not tried instead.
- **Token:** `tokens/NAME` next to `hosts.json`, 64 hex digits as `rusty-mux-ctl token create` prints them, a regular file that group and others cannot access. Never in `hosts.json`, never on the command line.
- **Connecting:** `rusty_mux_client::tls::Relay` binds a socket in a private temporary directory and relays each local connection to the server over TLS pinned to the certificate, sending the token first (32 raw bytes), so every client above it connects to a local socket as with the SSH forward. It checks the pin at start. Unix only.
- **Not built:** the terminal client's own `--host`, the GUI, an interactive trust prompt, syncing TLS hosts, and a `hosts pin NAME --re-pin HEX` verb (today, remove the host and add it again).
- **Tests:** `client/src/tls/tests.rs` (relay end to end against a TLS server that checks the token; changed certificate refused with both fingerprints), hosts file and CLI parsing, token file mode.
