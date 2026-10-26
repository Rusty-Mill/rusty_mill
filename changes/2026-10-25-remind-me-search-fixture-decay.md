---
category: Fixed
changelog: "A `remind_me_core` search test no longer starts failing a month after its fixture date."
---
## 2026-10-25 - remind_me search test no longer decays with the calendar

- **Fixed:** `keyword_search_ranks_filters_and_pages_the_corpus` built its corpus at fixed dates (2026-09-26/27), but the vitality floor it checks (`min_effective_vitality`) decays against the real clock. From 2026-10-25 row "f" fell below `VITALITY_FLOOR` and the test failed on `main` and in every queue entry that ran the remind_me jobs (first seen in #1075's queue run). The fixture now sets `decay_rate: 0.0`, so each row keeps the vitality it was written with and only "g"'s base weight (0.001) puts it under the floor, exactly as on the day the test was written. No product code changes.
