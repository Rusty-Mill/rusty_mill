---
category: Added
changelog: TLS native engine: smaller independent review scope (draft) and ADR-0002 Amendment 1 allowing one named consumer to opt in locally under the Tier 1 evidence bar; nothing wired.
---
## 2026-10-11 - TLS native engine: review scope draft and ADR-0002 Amendment 1

- **Added (docs):** `docs/research/TLS-REVIEW-SCOPE.md`, a draft smaller scope for the independent review (about 9,000 lines of client-path code, with the excluded areas and reasons). Awaiting the owner's approval. Found while scoping: `NativeTlsStream` negotiates TLS 1.2 as well as 1.3, so a 1.3-only scope needs either a small 1.3-only option on the stream or a wider scope; the owner decides.
- **Added (ADR):** Amendment 1 to the crate's ADR-0002. A consumer may use the gated engine types behind its own off-by-default feature, under the Tier 1 evidence bar, scoped in writing, with no change to the crate root or any default. The owner approved the approach on 2026-10-11; the text is for the owner to read before the first consumer is wired. Supersedes nothing in sections 1 to 6; Tier 2 still needs an explicit supersession.
- **Changed (docs):** `TLS-EVIDENCE-BAR.md` records the owner's 2026-10-11 answers: leave the weekly-schedule point, review scope drafted, ADR approach approved.
- **Known limitations:** no consumer is wired, the reviewer is not named, and the evidence bar is not met (no external certificate corpus, no long fuzz run, no review).
