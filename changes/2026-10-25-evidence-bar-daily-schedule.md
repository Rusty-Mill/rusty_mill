---
category: Changed
changelog: "The TLS evidence bar now says the scheduled CI run is daily, as `ci.yml` has scheduled it since 2026-10-21."
---
## 2026-10-25 - TLS evidence bar: scheduled run is daily

- **Changed:** `docs/research/TLS-EVIDENCE-BAR.md` called the scheduled CI run weekly in its engine-CI row and in decision 6. `ci.yml` has run it daily (03:17 UTC) since f34d63f7a, so Tier 1 item 1's 10 scheduled runs take about 10 days, not 10 weeks. Decision 6 and the criterion are unchanged; only the stated schedule is corrected. Raised in the automated review of #1023.
