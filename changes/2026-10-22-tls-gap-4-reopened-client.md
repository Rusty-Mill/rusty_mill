---
category: Changed
changelog: "Docs: the owner reopened gap 4 of the native crypto plan (the `rusty_tls` backend seam), client role only; signing and the server stay closed and no default changes."
---
## 2026-10-22 - Gap 4 of the native crypto plan reopened, client role only

- **Changed:** the owner decided `TLS-NATIVE-CRYPTO-SEAM.md` decision D1: "reopen gap 4, client role only". `CRYPTO-REPLACEMENT-PLAN.md` §0 records this as the implementer's transcription of the owner's words in the working session; it has not been restated from the owner's GitHub account. §13 and §13.2 D2, the gap 4 row, the seam note's status and D1 row, and the stale "(gap 4, closed)" reference in `AES-GCM-DESIGN.md` §5 are updated to match.
- **Not changed:** signing (gap 5), the server role and client certificates stay closed. No code, gate, default, dependency or consumer changes. Still open for the owner: the ADR that ADR-0002 §6 requires before the engine's primitives change (D2), signing under the native cfg (D3), suite order with portable AES (D4), and AES-GCM's review status (D5).
