---
category: Fixed
changelog: rusty_tokio's connect-refused tests no longer flake when another process grabs the closed port.
---
## 2026-11-02 - rusty_tokio: connect-refused tests hold their closed port

- **Fixed:** `tests/tcp_connect_refused.rs` picked a "closed" port by binding an ephemeral listener and dropping it, then connected to it. Anything else on the machine could bind and listen on that port in between. The connect then completed the handshake and came back as a success or as `ConnectionReset`, which was seen most often on the io-uring leg. The tests now keep the port bound, but not listening, for their whole run. A connect to it is still refused, and nothing else can claim it. The assertion is unchanged (#589).
