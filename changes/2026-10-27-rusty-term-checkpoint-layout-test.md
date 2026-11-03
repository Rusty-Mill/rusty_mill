---
category: Changed
changelog: rusty_term_core's RTCP v3 style-cache test now covers cells that are both wide and protected.
---
## 2026-10-27 - rusty_term_core: test every layout combination in the RTCP v3 style cache

- **Changed:** `a_capped_style_cache_writes_the_same_bytes` now writes runs that are wide and protected at once, as well as plain, wide-only and protected-only runs. It asserts that all four layout combinations appear in its lines. Review follow-up to #1153; no code change.
