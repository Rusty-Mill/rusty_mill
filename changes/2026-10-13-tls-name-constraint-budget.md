---
category: Security
changelog: rusty_tls native engine (opt-in only) bounds name-constraint checking with a comparison budget, so the x509-limbo nc-dos cases are refused like webpki does.
---
## 2026-10-13 - rusty_tls: bound name-constraint checking

- **Fixed:** `check_name_constraints_body` in the opt-in native engine did names times subtrees comparisons with no limit; limbo's `nc-dos-1`/`-2` (2,048 x 2,048) were accepted after over four million of them. It now refuses with `NameError::TooManyNameComparisons` above `MAX_NAME_COMPARISONS` (100,000), before comparing. The default rustls engine was never affected and nothing is wired.
- **Tests:** `name_constraints_over_the_comparison_budget_are_refused` (ordinary and just-under accepted; just-over and the limbo shape refused; mutation-checked). The limbo golden loses the two `looser` lines and nothing else.
- **Docs:** `TLS-EVIDENCE-BAR.md` states the worst case as a bound (about 9 x 10^7 comparisons at defaults), not a measured time. No known divergence from webpki remains open.
