# ADR-0089: `ORDER BY … DESC` on the Wire — `PageDesc` and `FilteredPageDesc` (Protocol 28)

- Status: **Proposed and implemented on one branch; awaiting the
  owner's review** (2026-09-21). Selected under the owner's standing
  "keep working the future-growth list" instruction as the "descending
  walks" open question carried since `ADR-0055`. A wire change, so —
  unlike `ADR-0076`–`ADR-0088` — not self-merged: the working agreement
  reserves public API changes for the owner.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-PAGE-DESCENDING-DESIGN.md` (the full
  design), `ADR-0055` (`Page`), `ADR-0059` (the `Ordered` index),
  `ADR-0068` (`FilteredPage`, protocol 26 — the precedent for a new
  request variant), `ADR-0061` (`ORDER BY`), `ADR-0043` (the Python
  client and the vectors), `SERVER-002` 0.17.0.
- Supersedes/Superseded by: none. Additive: `Request::PageDesc` (36),
  `Request::FilteredPageDesc` (37), `PROTOCOL_VERSION` 28,
  `PageBy::page_by_desc`, `ConnectionStore::page_desc`/
  `filtered_page_desc`, SQL `DESC`, `SchemaDrivenClient::page_desc`,
  Python `page_desc`/`filtered_page_desc`. No new response variant or
  error code.

## Context

Every page since `ADR-0055` is ascending; "the latest 50" has meant
walking the whole table forward. The sorted index reads backward as
cheaply as forward, and `ADR-0059` said so. The remaining future-growth
items are all wire changes; this is the most consumer-visible of them.

## Decision

Implement: two appended variants — `Page`/`FilteredPage`'s exact fields
with the cursor named `before`, walked from the greatest `(order_by,
id)` downward, strictly before the cursor, answered `Rows`; protocol
28, `Malformed` below it; `Ordered::page_by_desc` (`range(..cursor)
.rev()`), which `Memory`/`Relation`/`Reminder` use for their ordered
field while every other shape pages the scan's keys backward; the
filtered twin through the planner's candidate step and the descending
key selection (no descending bounded walk this round); `ORDER BY x
DESC` in SQL compiling to them; `page_desc` on both clients; the wire
spec, the golden vectors, and the fixture extended. Every descending
answer is the ascending answer reversed, proven in-process, over a
socket, and from Python.

The fork, for the owner:

- **(a) As implemented.** Two variants, the index walked backward.
- **(b) (a) plus a descending bounded walk** for the filtered twin.
- **(c) Decline and revert.** Protocol stays at 27.

## Consequences

- Positive (a): "latest N" on an ordered field costs the page
  (`due-latest-50` 73.2 µs against the scan-keyed
  `due-latest-scan-50` 54,154.4); `ORDER BY … DESC` parses; both
  clients carry it.
- Named, not hidden: a protocol bump — every client that wants `DESC`
  must say 28; the filtered twin is O(candidates), not O(page); the
  parser still reserves no keywords.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.74.0 /
  `FR-086` and `SERVER-002` 0.17.0; see the design's change history for
  the proof and the measurement. Builder: Claude, under the
  host-takeover convention; independent Codex inspection owed. **The
  PR is left open for the owner's review; nothing here merges without
  it.**
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 646 (up from 643), `server_sql_integration`
  62 (up from 61), `server_python_client` extended, the vectors
  fixture at 78, every other target unchanged and green, 937
  tests across 39 targets, 0 failed. Measured (`benches/server.rs`'s
  new `reminder-due` rows, 100K reminders over a real loopback socket,
  an idle 4-core Linux container): `due-latest-50` 73.2 µs,
  `due-latest-scan-50` 54,154.4 — `RESULTS.md`.
