---
category: Added
changelog: rusty-mux-server --shared can listen over TLS and serve peers that present an access token (#506, server side); rusty-mux-ctl token manages the tokens.
---
## 2026-11-13 - rusty_mux TLS listener, server side

- **Added:** `rusty-mux-server --shared ALLOW --tls ADDR:PORT --tls-cert PEM --tls-key PEM --tokens FILE` (Linux, card #506, rusty_term D235-D237). TLS 1.3 with the certificate the user supplies (PKCS#8 key). A peer sends its token, 32 raw bytes, first after the handshake; the token file names it `token:NAME`, and the allowlist must name that principal (`token:NAME` lines). A wrong, missing or unlisted token is held a second and closed before the protocol starts. A SIGHUP reload closes only the connections of tokens it rotated or revoked.
- **Added:** `rusty-mux-ctl token create|rotate|revoke NAME --file TOKENS`. The token is printed once as 64 hex digits; only its SHA-256 is stored (file mode 0600).
- **Added:** `rusty_mux_script::token` (the token file), `rusty_mux_server::Tls` and `Sharing::tls` (library API).
- **Known limitations:** no client side yet: pinning the server certificate's fingerprint needs a `rusty_tls` trust change, which waits for its own go. A TLS guest has no UDP and passes no descriptors; transport shows as `socket`. No wire change. New workspace edges only (`rusty_tls` and `rusty_base64` into the server, token deps into `rusty_mux_script`); no third-party dependency.
