---
category: Security
changelog: rusty_tls native engine (opt-in only) accepted three kinds of certificate that webpki refuses, found by a new x509-limbo differential; fixed with tests, and the differential now runs in CI.
---
## 2026-10-11 - rusty_tls: x509-limbo certificate differential finds and fixes three looser-than-webpki defects

- **Added:** `tests/handrolled_limbo.rs` runs 9,777 x509-limbo cases through the native path validator and webpki side by side. A ratchet file (`tests/data/limbo_divergences.txt`) fails the test on any new or vanished divergence. CI runs it in the `rusty-tls-engine` job through `limbo_differential.sh`, which pins the corpus by sha256 and skips with a visible warning (never silently) if it cannot be fetched or has changed upstream. `rusty_json` becomes a dev-dependency of `rusty_tls` for reading the corpus.
- **Fixed (security, opt-in engine only):** an intermediate CA whose `extendedKeyUsage` excluded server authentication was accepted as an issuer of server certificates; an end-entity certificate asserting `cA` was accepted; under a name constraint, a malformed `subjectAltName` entry was skipped and the certificate accepted. The default rustls engine was never affected. Each fix has a unit test that fails with it reverted, and reverting the end-entity fix is also caught by the ratchet.
- **Changed (CI planner):** `limbo_differential.sh` is one more file whose change selects the TLS engine jobs.
- **Findings recorded:** `docs/research/TLS-EVIDENCE-BAR.md` lists every remaining divergence. One owner decision: two name-constraint DoS cases (`pathological::nc-dos-1`, `-2`) are accepted by this validator and refused by webpki.
- **Known limitations:** neither engine implements CRLs or the `pedantic` issuance lints; 25 limbo cases are not compared (client validation, custom key usage, max depth, unparsable time, no peer name). The corpus is pinned by content hash on a moving upstream branch, so the pin needs a manual bump when upstream changes. The step has not yet run on a GitHub runner.
