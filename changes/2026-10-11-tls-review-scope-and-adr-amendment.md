---
category: Added
changelog: TLS native engine: smaller independent review scope (draft) and proposed ADR-0002 Amendment 1 (not accepted, not operative); nothing wired.
---
## 2026-10-11 - TLS native engine: review scope draft and proposed ADR-0002 Amendment 1

- **Added (docs):** `docs/research/TLS-REVIEW-SCOPE.md`, a draft smaller scope for the independent review (about 9,000 lines of client-path code, with the excluded areas and reasons). Awaiting the owner's approval. Found while scoping: `NativeTlsStream` negotiates TLS 1.2 as well as 1.3, so a 1.3-only scope needs either a small 1.3-only option on the stream or a wider scope; the owner decides.
- **Added (ADR, proposed):** Amendment 1 to the crate's ADR-0002, marked Proposed and not operative. It proposes that a consumer may use the gated engine types behind its own off-by-default feature, under the Tier 1 evidence bar, scoped in writing, with no change to the crate root or any default. Approach and text await the owner's confirmation. `anyExtendedKeyUsage` and the `nc-dos` cases are listed as unresolved pre-wiring blockers. Supersedes nothing in sections 1 to 6; Tier 2 still needs an explicit supersession.
- **Changed (docs):** `TLS-EVIDENCE-BAR.md` records the weekly-schedule "leave it" and the ADR approach as pending confirmation; the Tier 1 bar is unchanged.
- **Known limitations:** no consumer is wired, the reviewer is not named, and the evidence bar is not met (`anyExtendedKeyUsage` and `nc-dos` unresolved, no long fuzz run, no review).
