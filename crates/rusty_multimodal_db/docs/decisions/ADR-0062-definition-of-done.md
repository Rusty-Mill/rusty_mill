# ADR-0062: Definition of Done

- Status: **Accepted** (2026-09-14 — the owner accepted as proposed, no
  changes requested)
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/charter/CHARTER.md` (the original, already-met "Done" —
  the three-backend hypothesis test), `docs/FUTURE-GROWTH.md` ("Path to
  SQLite/DuckDB parity" — explicitly named as multi-year, not a
  destination, and explicitly out of scope here), `docs/reports/
  2026-09-07-hub-spike-report.md` (the evidence base for items 4–6
  below), `ADR-0060` (the crash-atomicity gap item 4 closes), `ADR-0043`
  (the client ecosystem the differential-suite port in item 5 exercises).
- Supersedes/Superseded by: none. Docs-only — no code change.

## Context

This crate's charter defined one "Done": test the UUID-canonical-store-
as-views hypothesis against AoS/SoA baselines. That bar was met years
ago (`RESULTS.md`). Since then, every round has been consumer-driven
(`rusty_remind_me`/`rusty_remindme_mcp`, per the owner's own standing
mandate quoted in `docs/PROJECT-STATUS.md` items 133+) with no new
endpoint ever defined — the charter's own non-goals ("no server, no
persistence, no network surface") were superseded one ADR at a time,
each recorded, but never replaced with a new stated destination.
`docs/FUTURE-GROWTH.md` is explicit that full SQLite/DuckDB parity is
"a different tier of project... roughly a multi-year effort," not
something this project is committed to reaching.

Asked directly whether a roadmap to "Done" exists, the honest answer
was no. This ADR is that roadmap.

## Decision

Define "Done" as: **a solid backend for `rusty_remind_me` and sibling
`rusty_*` consumers on Linux — not a SQLite/DuckDB competitor.**

Already met, verified rather than assumed:

1. Every `HubStore` method the 2026-09-07 spike exercised has a real
   backend primitive — all five gaps that spike named are closed
   (`ADR-0054`–`0058`).
2. CI gates every push on the real target platform (`ubuntu-latest`):
   `fmt`, `clippy` across three feature sets, the full test suite, an
   MSRV build, and the Python client's offline conformance vectors.
3. Durability and concurrency claims are measured against a real
   process restart and real concurrent load, not assumed from the
   design alone (the spike report's own restart-and-reopen section).

Still open — the concrete, bounded remainder:

4. **Crash-atomic `WriteBatch`.** `ADR-0060`'s atomic mode is
   precondition- and isolation-atomic but not crash-atomic across a
   mid-batch process failure — a real, already-named follow-on, not a
   new finding.
5. **A differential test suite port.** The hub spike's own 14-method
   coverage suite honestly asserted every gap but was explicitly not a
   full port of the existing ~60-test SQLite/Postgres suites — flagged
   there as follow-up work, not done here.
6. **The entity-id mapping decision.** The spike's zero-pad-and-strip
   workaround for the consumer's 48-bit entity id into this crate's
   128-bit `Uuid` was named as "a bigger, cross-repo decision" the
   spike explicitly declined to make. Still undecided.
7. **No regression on the correctness bar the fix session just
   established.** Not a new work item — a standing condition: the two
   real bugs found and fixed this session (`insert_log`'s Windows
   `sync_data` handle, `dog_server`'s test-only drive-letter collision)
   stay fixed.

Explicit non-goals, named so they are never silently chased later: a
SQL query planner or cost-based optimizer, a general MVCC transaction
manager, `ALTER TABLE`-style dynamic schema, a `NULL` concept, and
arbitrary-predicate (non-relation) joins. All five remain tracked in
`docs/FUTURE-GROWTH.md` as directions this project *could* take, not
commitments this ADR makes.

## Consequences

- Positive: "Done" is now four bounded, named work items (4–6, plus
  holding 7) instead of an open-ended chase of `FUTURE-GROWTH.md`'s
  "big three." A future session can pick one, finish it, and cross it
  off — the roadmap this ADR exists to provide.
- Named, not hidden: this is a scope *narrowing* relative to
  `FUTURE-GROWTH.md`'s own broader framing of what SQL/DuckDB parity
  would require. Reopening that broader scope is a future, separate
  decision, not implied by anything here.
- Item 6 (id-mapping) is explicitly cross-repo — closing it needs a
  decision in `rusty_remind_me` too, not just here. Named as a
  dependency, not silently assumed to be this crate's call alone.

## Acceptance and implementation

- 2026-09-14: proposed and accepted in the same session, no changes
  requested. Docs-only; items 4–6 tracked as their own roadmap entries
  (`docs/roadmap/ROADMAP.md`) for future rounds to pick up.
