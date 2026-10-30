---
category: Added
changelog: docs/projects/mux/UDP-TRANSPORT.md, the design (not code) for rusty_mux's own UDP transport, for review before any implementation.
---
## 2026-10-30 - UDP transport design note

- **Added:** `docs/projects/mux/UDP-TRANSPORT.md` designs Phase 3 step 4 (C13), roaming over our own UDP datagrams. The server hands each `--ssh` client a random key over the SSH-authenticated connection, so the planned `rusty_tls` exporter is no longer needed. Datagrams are sealed with ChaCha20-Poly1305 from `rusty_aead`, with a per-direction sequence nonce and a replay window. The server replies to the source of the newest authenticated datagram, so a roam costs no handshake, and it never answers an unauthenticated one. A small ARQ carries the unchanged mux frame stream. A new UDP connection takes the client over from the SSH one, and the client falls back to SSH when UDP is blocked. The note also has a test plan, four open questions for the owner and a build order.
- **Known limitations:** design only; no code, wire format or feature bit changes.
