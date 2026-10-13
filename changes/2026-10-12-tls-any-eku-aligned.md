---
category: Security
changelog: rusty_tls native engine (opt-in only) no longer treats anyExtendedKeyUsage as a wildcard, matching webpki.
---
## 2026-10-12 - rusty_tls: align anyExtendedKeyUsage with webpki

- **Fixed:** `check_eku` in the opt-in native engine accepted `anyExtendedKeyUsage` as satisfying `serverAuth`; webpki refuses it. Now refused on the leaf and every CA (`PathError::RequiredEkuMissing`), by owner decision. The default rustls engine was never affected and nothing is wired.
- **Tests:** the former pinned divergence is now an agreement test against webpki; the leaf expectation is reversed. Mutation-checked (restoring the wildcard fails both).
- **Docs:** `TLS-EVIDENCE-BAR.md`, `TLS-REVIEW-SCOPE.md` and ADR-0002 Amendment 1 condition 6 now list only the `nc-dos` cases as an open pre-wiring blocker.
