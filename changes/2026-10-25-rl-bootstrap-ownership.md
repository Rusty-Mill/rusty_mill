---
category: Security
changelog: rl_live_test refuses delegated Epic bootstrap ownership instead of risking termination of an independently launched game.
---
## 2026-10-25 - Fail closed on uncorrelated Epic bootstrap launches

- **Fixed:** bootstrap adoption requires a direct child of the retained, still-live launch process. Epic parentage, recent creation and a matching shared log are identity evidence only and cannot authorize adoption or termination. Native-handle identity checks remain in place.
- **Validation:** mocked production-session regression covers an independent Epic launch during polling with all previous checks satisfied, both before and after the retained launch process exits; it is never owned or killed.
- **Known limitations:** ordinary Epic delegation aborts and requires explicit operator bootstrap cleanup. No Windows desktop or live-game verification was performed.
