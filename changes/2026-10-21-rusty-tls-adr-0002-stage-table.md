---
category: Fixed
changelog: "`rusty_tls`: ADR-0002's stage table now shows TLS 1.2 (stage 4b) as landed rather than declined, and lists the later stages."
---
## 2026-10-21 - rusty_tls: ADR-0002 stage table brought up to date

- **Fixed:** ADR-0002's stage table still read "4b: TLS 1.2 declined, not built", but the owner approved TLS 1.2 on 2026-10-08 (decision D1) and the engine has had `record12`, `schedule12`, `client12`, `server12` and `negotiate` since then. Row 4b now says it was declined, reopened and landed. A row for stages 7 to 14 was added, along with a dated status note saying which prose under the table is history: the "Why 4b is declined" paragraphs, and stage 5's "does not do" list, where only 0-RTT still holds. The `handrolled` module doc said only the TLS 1.2 record layer and key derivation existed; it now points to the finished stages. `docs/research/TLS12-DESIGN.md`'s status line still said the server, version negotiation and hardening were unbuilt; it now says the TLS 1.2 work is done and that superseding ADR-0002 (D3) is still open.
- **Not changed:** §3 (never the default), both gates, and all engine code. The historical prose is annotated, not rewritten.
