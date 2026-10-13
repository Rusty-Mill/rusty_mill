---
category: Changed
changelog: rusty_tls native engine (docs only): two limbo cases where it follows RFC 5280 and webpki is stricter are recorded as proposed exceptions, pending owner confirmation.
---
## 2026-10-13 - rusty_tls: propose two RFC 5280 exceptions to webpki

- **Changed (docs):** `TLS-EVIDENCE-BAR.md` records `pathlen::self-issued-certs-pathlen` (RFC 5280 §6.1.4(l)) and `rfc5280::nc::permitted-self-issued` (§6.1.3(b)/(c)) as proposed exceptions where the engine follows the standard and webpki is stricter. Basis: the user's direction in the working session, "Always err on the side of the standards". Both remain pre-wiring blockers until the owner confirms them directly on the PR, and an approval would cover exactly these two cases; limbo expecting success is not the acceptance. ADR-0002 Amendment 1 condition 6 (still Proposed and non-operative) points at it.
- **Not changed:** no code, no golden, no default or gate. The `anyExtendedKeyUsage` alignment with webpki (#628) stands: RFC 5280 §4.2.1.12 permits restricting it, so favoring the standards does not require reversing it.
