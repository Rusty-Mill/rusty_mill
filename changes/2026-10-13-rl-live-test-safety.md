---
category: Fixed
changelog: Harden RL live-test report scrubbing, owned-process cleanup, watcher startup and analyzer failures; stream demolition analysis and reject decode gaps.
---
## 2026-10-13 - RL live-test review corrections

- **Fixed:** Consume complete Rust Debug names with exact bot labels; retain process handles and fail closed on ambiguous Epic ownership; preserve watcher arguments and verify readiness; report each analyzer exit status.
- **Fixed:** Fold demolition recordings one bounded frame at a time and reject malformed/truncated data before returning a lifecycle report.
- **Validation:** Synthetic Rust recordings and a dependency-free mocked PowerShell suite, wired into the required CI gate. No real game, bots, desktop automation or watcher is launched by tests.
- **Known limitations:** Windows desktop integration is not exercised in cloud validation. Epic delegation through an existing launcher cannot establish ownership and now aborts without killing that game. Historical live findings remain author-reported; login/session causes are unproven.
