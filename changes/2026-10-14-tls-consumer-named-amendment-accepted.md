---
category: Changed
changelog: rusty_tls native engine (docs only): ADR-0002 Amendment 1 accepted and rleval-app's OIDC transport named as the one opt-in consumer; nothing wired. Also a comment-only RFC citation fix and a retry around the BoGo suite's BoringSSL fetch.
---
## 2026-10-14 - rusty_tls: first consumer named, Amendment 1 accepted, two small fixes

- **Changed (docs):** the owner accepted ADR-0002 Amendment 1 and named `rleval-app`'s OIDC transport (client only, TLS 1.3 only, named hosts) as the one opt-in consumer, in the working session on 2026-10-14. Recorded in the ADR and `TLS-EVIDENCE-BAR.md`. **Nothing is wired**: the Tier 1 bar is not met (independent review and the long fuzz run are open, as is the 10-scheduled-run live evidence), and `NativeTlsStream` still negotiates TLS 1.2, so before wiring, TLS 1.3-only operation must be enforced (a 1.3-only option) or the owner must separately approve widening the protocol scope to TLS 1.2 with a review of it.
- **Fixed (comments only):** the self-issued name-constraint exemption cites RFC 5280 §6.1.3(b)/(c), not §6.1.4(b), in `name.rs` and `handrolled_path.rs`.
- **Fixed (CI script):** `rusty_tls/bogo/run.sh` retries the pinned-commit `git fetch` up to five times with a growing pause, so a transient 503 from googlesource no longer fails the job (it did once, in a merge-queue run). A persistent failure still fails. Covered by `.github/scripts/test_bogo_fetch_retry.py`.
