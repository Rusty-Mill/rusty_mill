---
category: Added
changelog: docs/projects/mux/UDP-TRANSPORT.md, the design (not code) for rusty_mux's own UDP transport, for review before any implementation.
---
## 2026-10-30 - UDP transport design note

- **Added:** `docs/projects/mux/UDP-TRANSPORT.md` designs Phase 3 step 4 (C13), roaming over our own UDP datagrams. The server hands each `--ssh` client a random key over the SSH-authenticated connection, so the planned `rusty_tls` exporter is no longer needed. Datagrams are sealed with ChaCha20-Poly1305 from `rusty_aead`, with a per-direction sequence nonce, a direction check and a replay window. The server replies to the source of the newest authenticated datagram, so a roam costs no handshake, and it never answers an unauthenticated one. One association per client process carries a stream per mux connection, each a small ARQ with flow control over the unchanged frame stream, so new panes need no new SSH login. After an authenticated probe, each connection takes its client over onto UDP, with SSH fallback when UDP is blocked and #1251 token recovery if the path dies mid-move. The note also has a test plan, four open questions for the owner and a build order.
- **Known limitations:** design only; no code, wire format or feature bit changes.
