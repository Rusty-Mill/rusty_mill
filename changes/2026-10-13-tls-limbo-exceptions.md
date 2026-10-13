---
category: Changed
changelog: rusty_tls native engine (docs only): two limbo cases where it follows RFC 5280 and webpki is stricter are recorded as accepted exceptions (owner acceptance recorded 2026-10-13).
---
## 2026-10-13 - rusty_tls: record two accepted RFC 5280 exceptions to webpki

- **Changed (docs):** `TLS-EVIDENCE-BAR.md` records `pathlen::self-issued-certs-pathlen` (RFC 5280 §6.1.4(l)) and `rfc5280::nc::permitted-self-issued` (§6.1.3(b)/(c)) as accepted exceptions where the engine follows the standard and webpki is stricter. Basis: the user's direction in the working session, "Always err on the side of the standards". Accepted by the owner in the working session on 2026-10-13 ("Confirmed, I accept both exceptions"), covering exactly these two cases: no consumer wiring, no broader standards-preference policy, no reversal of #628, no unrelated ADR. Limbo expecting success is not the acceptance. ADR-0002 Amendment 1 condition 6 (still Proposed and non-operative) points at it.
- **Not changed:** no code, no golden, no default or gate. The `anyExtendedKeyUsage` alignment with webpki (#628) stands: RFC 5280 §4.2.1.12 permits restricting it, so favoring the standards does not require reversing it.
