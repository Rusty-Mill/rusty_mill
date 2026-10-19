---
category: Changed
changelog: **BBP criteria prices checked.** `prices_as_of` in `docs/bbp/2b-criteria.json` is now 2026-10-19, after the owner compared the Opus 5.5 and Sonnet 5.5 rows with the Claude Platform Docs model pricing page.
---
## 2026-10-19 - BBP criteria: prices checked against the price page

- **Changed:** `prices_as_of` is `2026-10-19`. All five prices (input, output, 5m writes, 1h writes, cache hits) for `claude-opus-5-5` and `claude-sonnet-5-5` match the page; no price changed.
- Haiku stays unpriced and refused; the page shows a long-context tier for it.
- `status` is unchanged: the criteria and the run remain unapproved.
- Data only: no engine, schema or log path.
