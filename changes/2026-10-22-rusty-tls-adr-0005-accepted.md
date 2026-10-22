---
category: Changed
changelog: "`rusty_tls`: ADR-0005 accepted: the hand-rolled engine's client path may use first-party primitives instead of `ring`, behind its own feature and cfg, with signing compiled out and ChaCha20-Poly1305 first."
---
## 2026-10-22 - rusty_tls: ADR-0005, first-party primitives for the hand-rolled client engine

- **Changed:** the owner approved decisions D2 to D4 of `docs/research/TLS-NATIVE-CRYPTO-SEAM.md` ("Approved", in the working session; implementer's transcription). The plan's §10 draft is now `crates/libs/net/rusty_tls/docs/adr/0005-first-party-primitives-for-the-client-engine.md`, accepted for the client role only. It supersedes ADR-0002 §6 for the engine's client-path primitives. Under the new `native-crypto` feature and `--cfg rusty_tls_native_crypto` (not yet built), signing, the servers, tickets and client certificates are compiled out, and ChaCha20-Poly1305 is offered first. ADR-0002 §6, the plan's §0 and §10, and the seam note's decision table now point to it.
- **Not changed:** no code, gate, default, dependency or consumer. `ring` stays the engine's default backend and the only backend of `rustls`. D5 (AES-GCM's review status) is still open and must be settled before the native backend merges.
