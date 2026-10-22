---
category: Added
changelog: "Docs: `docs/research/TLS-NATIVE-CRYPTO-SEAM.md` scopes gap 4 of the native crypto plan, taking the hand-rolled TLS engine off `ring`; no code, the gap is still closed."
---
## 2026-10-21 - Scope for taking the native TLS engine off `ring` (gap 4)

- **Added:** `docs/research/TLS-NATIVE-CRYPTO-SEAM.md`, the scope for gap 4 of `CRYPTO-REPLACEMENT-PLAN.md`. It lists every `ring` call in `rusty_tls`'s hand-rolled engine against the first-party replacement for it, which covers everything except signing. It also covers the design: one internal backend module, selected by a cargo feature plus a `--cfg` flag in the style of ADR-0002 §1, with signing compiled out so the engine is client-only and ring-free. The rest is the evidence plan, effort (about 6 to 9 days) and five decisions for the owner. Plan §13.1's gap 4 row now links to it.
- **Not changed:** no code, gate, default, dependency or consumer. Gap 4 stays closed until the owner reopens it, and rustls keeps `ring`.
