---
category: Fixed
changelog: Resolve watchdog sampled first-frame PCs without return-address offsets and stabilize the strict source-location fixture.
---
## 2026-10-24 - Watchdog sampled source locations

- **Fixed:** Linux stack-dump first frames resolve their sampled instruction address, compensating for backtrace's internal return-address adjustment. Named frames include the raw IP for diagnostics. The CPU-bound fixture avoids checked-counter line-0 spans, and its test requires a positive source line on the stuck frame itself.
- **Evidence:** `crates/apps/rusty_remind_me/docs/research/WATCHDOG-SOURCE-LOCATIONS.md` records tested-merge/base runs, disassembly, dependency behavior and controlled reproduction.
- **Known limitations:** The original CI output omitted its PC, so its exact sampled instruction remains unknown. Arbitrary compiler-generated spans can have no source line; caller-frame/signal handling still follows rstack-self.
