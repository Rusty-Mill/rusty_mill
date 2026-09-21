# ADR-0091: A Contradictory Filter Is `Malformed` (Protocol 29)

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-21). `ADR-0083`'s option (b), taken on the owner's word
  ("contradiction") as a protocol round (28 → 29).
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-CONTRADICTION-DESIGN.md` (the full
  design), `ADR-0083` (whose option (b) this is), `ADR-0075` (the
  guarded walk that answered nothing), `ADR-0022` (rule 3; version 27
  as the precedent), `SERVER-002` 0.18.0.
- Supersedes/Superseded by: none. Additive: `protocol::contradicted`,
  `serve::request_contradicted`, one guard arm, `PROTOCOL_VERSION` 29.
  No new variant or error code.

## Context

A filter no record can satisfy answered nothing since `ADR-0075` —
indistinguishable from an empty table. It is a mistake in the query,
and `Malformed` is what every other mistake in a request gets. Changing
that answer is wire-visible, so it is versioned.

## Decision

Implement: `contradicted(filter)` — on one field, two `Eq` with
different literals, an `Eq` beside an `Ne` of the same literal, or (on
`U32`/`I64`) a lower bound above an upper bound or equal to it with
either side exclusive — pure over the literals; `handle_connection`
refuses `Query`, `Aggregate`, `FilteredPage`, `FilteredPageDesc`, and
either side of `Join` carrying one with `Malformed` before any read on
a connection negotiated at 29 or above; below 29 the empty answer
stands (rule 3's nearest older shape, as version 27 applied it to a
flag bit). Proven by a unit matrix and over a socket at 29 and at 28.

## Consequences

- Positive: a mistyped clause is an error, not a silent empty result;
  no client below 29 changes behaviour.
- Named, not hidden: soundness only — a filter can be empty against a
  table without being a contradiction; clients do not pre-check.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8`
  as `SERVER-001` v0.76.0 / `FR-088`, `SERVER-002` 0.18.0. `cargo fmt
  -p rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 647 (up from 646), `server_sql_integration`
  64 (up from 63), 940 tests across 39 targets, 0 failed; the
  Python vectors suite green. Four pre-existing socket assertions that pinned the old contract ("empty, never an error") are rewritten to `Malformed` and named: the inverted and equal-exclusive ranges in `range_query_on_the_ordered_field_matches_the_full_scan_on_memory_and_relation` (`ADR-0075`), the inverted range in `since_shaped_filtered_page_walks_the_index_and_returns_the_exact_sequence` (`ADR-0076`), the inverted-range count in `a_pure_range_count_from_the_index_matches_the_decoded_count_exactly` (`ADR-0081`), and the `= 3000 AND > 3000` case in `redundant_and_contradictory_bounds_on_the_range_field_answer_exactly` (`ADR-0083`); no other pre-existing test changed but the version pins. Not measured: nothing is read. Builder:
  Claude; independent Codex inspection owed.
