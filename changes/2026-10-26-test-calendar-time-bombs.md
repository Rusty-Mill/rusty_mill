---
category: Fixed
changelog: Six tests no longer start failing when the calendar passes their fixture dates (the first on 2026-11-02).
---
## 2026-10-26 - Tests that would fail as the calendar moves on

A scan for fixtures dated against code that reads the real clock (after #1086) found six more. Each fix is test-only and was checked with `faketime`: the old tests fail at the shifted date and the fixed ones pass.

- **Fixed (breaks 2026-11-02 ~22:58 UTC):** `remind_me_core` `context_test::search_filters_by_each_scope_field` searched a fixture dated 2026-10-04 through the default vitality floor, which decays against `Utc::now()`. Its helper now sets `include_dormant: true`; the tests are about scope, not decay.
- **Fixed (breaks 2030):** `remind_me_core` `sync_test` and `graph_sync_test` used a 2030 "newer" timestamp against a local row stamped now, and `rusty-hister-model` `session` tests used 2030 expiries filtered against now. These fixtures move to 2099, keeping their order.
- **Fixed (breaks 2030):** the `rusty_tls` server12 and negotiate socket interop suites issue certificates expiring 2030-01-01 that OpenSSL verifies against the real clock; they now expire in 2100. Suites that inject the verification time are unchanged.
