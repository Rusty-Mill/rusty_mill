---
category: Added
changelog: rusty_mux spike 4 report on the roaming transport for Phase 3 (TCP and rusty_tls with resume first, own UDP datagrams later, no QUIC).
---
## 2026-10-28 - rusty_mux spike 4: roaming transport

- **Added:** `docs/projects/mux/SPIKE-4-ROAMING.md`, the spike 4 report. A loopback prototype moved a client to a new address over TCP and resumed its events and screen, and roamed authenticated UDP datagrams with no handshake. It recommends TCP with `rusty_tls` and resume first, then our own UDP datagrams for mosh-like feel, and not QUIC.
- **Known limitations:** report only; the prototype is not merged. It found that `rusty_tokio`'s `LocalRuntime` delays every `spawn_local` wake by up to 50 ms (new risk R16), and that a reconnecting client loses its input lease to its old connection.
