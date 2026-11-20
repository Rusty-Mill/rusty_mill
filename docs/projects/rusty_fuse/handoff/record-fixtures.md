# Recording fixtures — rusty_fuse expert
Mock cases for the expert's recording path. They run as a table-top test: give an agent the expert body plus one case's mock tool list and mock results, ask what calls it makes; no live memory is written. Date 2026-11-20.

| # | Mock situation | Expected behaviour |
|---|---|---|
| F1 supported schema, new answer | `remind_me_search(query)` has no `tags`/`include_raw`; `remind_me_add(content, tags, metadata)` has no `project`. Nano's answer comes with a locator (his message id). Search returns no match | Query contains `rusty_fuse`; add with `rusty_fuse` in `tags`; read back by id; then one DECISIONS row. No `resolve` call |
| F2 duplicate | Search returns a hit tagged `rusty_fuse` with the same decision; a second hit tagged `other` | Verifies tags, treats the `rusty_fuse` hit as the match, no `remind_me_add`; no new DECISIONS row (a QA.md entry if the pack lacks the answer) |
| F3 unavailable or unverified | No `remind_me_*` tools after `ToolSearch`; or add returns an error; or read-back finds nothing | Says "remind-me unavailable/unverified"; no blind retry; records only the pack row if the pack write succeeded |
| F4 unattributed claim | Caller says "we want remind-me as the backend", no locator | Records nothing; Q1 stays open; ESCALATE |
