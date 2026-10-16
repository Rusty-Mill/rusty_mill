---
category: Changed
changelog: rusty_tls native engine (docs only): the Tier 1 review scope is approved and a reviewer is named. The review has not started, so the evidence bar is still unmet; nothing is wired.
---
## 2026-10-16 - rusty_tls: review scope approved, reviewer named

- **Changed (docs):** the owner approved the smaller review scope in `TLS-REVIEW-SCOPE.md` (client path, about 9,000 lines) and nominated David Bailey as the reviewer (independence is not settled), in the working session on 2026-10-16. The TLS 1.3-only question is answered by option 1 (`NativeTlsStream::tls13_only()`, merged in #716), so the four TLS 1.2 client files stay out of scope provided the consumer sets that option.
- **Still open:** the review itself (not started), its time box, and the fuzz long-run length (the reviewer's to set). Tier 1 item 4 stays unmet, as do the long fuzz run and the 10 scheduled live runs. **Nothing is wired.**
