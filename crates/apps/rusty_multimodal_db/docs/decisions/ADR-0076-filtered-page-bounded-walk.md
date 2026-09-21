# ADR-0076: The O(page) Bounded Walk for a Range-Filtered `FilteredPage`

- Status: **Proposed and implemented on one branch** (2026-09-20; the
  `ADR-0059` precedent). Selected under the owner's standing "keep
  working the future-growth list" instruction as `ADR-0075`'s own
  option (b); the fork below is held open for the owner at review.
- Date: 2026-09-20
- Deciders: baileyrd
- Related: `docs/design/SERVER-FILTERED-PAGE-WALK-DESIGN.md` (the full
  design), `ADR-0075` (whose option (b) this is — "a `since`-shaped
  `FilteredPage` still reads every in-range record before `page_rows`
  cuts the page"), `ADR-0068` (`FilteredPage`, "a trait method with a
  shared default so a domain *could* later narrow candidates more
  cheaply"), `ADR-0059` (`Ordered`/`page_by`, reused unchanged),
  `ADR-0055` (`Page`, whose consistency class the walk inherits),
  `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive — the `filtered_page`
  default body factored into a free function, two helpers, two adapter
  overrides; no generic, wire, protocol, `FieldCapabilities`, or client
  change.

## Context

`ADR-0075` made a `WHERE` range on `Memory`/`Relation`'s
`updated_at_unix_ms` walk the `Ordered` index in every filtered read,
and named plainly that `FilteredPage` still reads every in-range record
before `page_rows` cuts the page — so the `since`-shaped listing (`WHERE
updated_at > since ORDER BY updated_at LIMIT 50`), whose bound admits
most of the table, pays *k* ≈ *n* reads for 50 rows. The unfiltered
`Page` by the same field already costs the page (`page_by(cursor,
limit)`, `ORD-FR-005`).

Read against the code: when `order_by` is the range field and every
predicate is a bound on that same field, the filtered page is the
unfiltered walk from a composed cursor — the later of the client's
cursor and the tightest lower bound — cut at the first row an upper
bound rejects, which (ascending on the very key the bounds are on) is
where every later row would be rejected too. No generic change: `page_by`
is already the limited walk. A Rust default method body cannot be
called from an override, so the default's body moves into a free
function the override falls back to for every ineligible shape.

## Decision

Implement: `filtered_page_by_candidates` (the former default body),
`bounded_walk_applies`, `bounded_walk_start`, and `bounded_filtered_page`
in `serve.rs`; `filtered_page` overrides on `Memory` and `Relation`
that take the walk when eligible and the factored default otherwise.
The eligible page returns the identical sequence the default returns,
proven in-process against that exact oracle and over a socket via SQL.

The fork, held for the owner:

- **(a) As implemented.** Eligible = `order_by` is the range field and
  every predicate is a non-`Ne` bound on it. The page costs the page.
- **(b) (a) plus walk-past-rejects for mixed filters** (`… AND
  category = 'x'`): keep walking until `limit` rows match — O(page)
  typically, O(*k*) worst case. A second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the `since`-shaped filtered listing costs what the
  unfiltered `Page` costs; `docs/FUTURE-GROWTH.md` drops "the O(page)
  bounded walk … not taken".
- Named, not hidden: a third code path answers `FilteredPage` on two
  domains; each is proven equal to the next.
- Named, not hidden: the eligible page has `Page`'s consistency class,
  not the default's — an id deleted between the walk and its read
  shortens the page by one instead of being back-filled; a client
  re-issues from the last cursor, as for `Page`.
- Named, not hidden: a mixed filter still pays the `ADR-0075` cost —
  option (b).

## Acceptance and implementation

- 2026-09-20: proposed and implemented on
  `claude/planner-filtered-page-walk` as `SERVER-001` v0.61.0 /
  `FR-073`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-20: implemented, same branch, no deviation. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 617 (up from 614), `server_sql_integration` 52 (up from 51), every other target unchanged and green, 896 tests across 39 targets, 0 failed.
  Measured (acceptance criterion 5, `benches/server.rs`'s new `memory-planner` `index-since` row — `FilteredPage` `WHERE updated_at_unix_ms >= 1000 ORDER BY updated_at_unix_ms LIMIT 50`, a bound admitting ~99% of the same 100K `Memory` table, vs. the `created_at_unix_ms` full-scan control, over a real loopback socket, the row added and measured on the pre-change code first): before this round 123,436.1 µs (walk every in-range record, then page; control 146,098.2 µs), after 89.4 µs (the bounded walk; control 143,975.6 µs) — ~1,380× — `RESULTS.md`. The fork above remains the owner's at review;
  (a) is what merged if the PR merges unchanged.
- 2026-09-21: merged unchanged as PR #276 under the owner's `/goal` to merge it and keep working the future-growth list — option (a) confirmed by the merge; option (b) taken the same day as `ADR-0077` (`docs/design/SERVER-FILTERED-PAGE-WALK-MIXED-DESIGN.md`), which widens this walk's eligibility and its cursor helper and names, not hides, that this ADR's eligibility/start-cursor unit tests were rewritten for the new contract.
