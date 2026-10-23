---
category: Security
changelog: Hold review-router activation and isolate read-only generation from exact-head review publication.
---
## 2026-10-22 - Isolate review generation and publication

- **Fixed:** reject fork/draft PRs; use explicit read-only Claude GitHub authentication and constrained review tools; run Codex with read-only permissions and dropped privileges.
- **Security:** publish bounded untrusted text from a fresh runner using an ephemeral narrow token and exact PR/base head checks; disable every job with a literal gate and pin actions.
- **Known limitations:** provider, identity-routing, runtime-version and spending decisions remain deferred; no provider runtime was invoked.

- **Activation hold:** keep the complete design in an inert template and a disabled live hold stub because current repository action policy blocks provider actions even in skipped jobs. A separate allowlist decision is required; no rules were changed.
- **Fixed:** make the live hold manual-only so it cannot introduce a PR-triggered check outside `ci.yml` and `required-gate`; retain future PR triggers only in the inert template and preserve the existing CI-policy test.
