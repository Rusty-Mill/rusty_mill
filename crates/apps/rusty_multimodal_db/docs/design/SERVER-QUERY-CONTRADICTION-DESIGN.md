# Server Query: A Contradictory Filter Is `Malformed` (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-21, `ADR-0091`). The last future-growth item the owner was
  offered — `ADR-0083`'s option (b) — taken on the owner's word
  ("contradiction"), as a protocol round (28 → 29) because it changes
  what an existing client sees.
- Date: 2026-09-21
- Related: `ADR-0083`/`docs/design/SERVER-QUERY-PLANNER-BOUND-TIGHTENING-DESIGN.md`
  (whose option (b) this is — "`a = 3 AND a > 3` as `Malformed` before
  any walk; a wire-visible behavior change; a protocol round if ever
  wanted"), `ADR-0075` (the guarded range walk that answered a
  contradiction with nothing), `ADR-0022` (the version rules; version
  27 as the precedent for a semantics change with no new variant),
  `ADR-0089` (protocol 28), `SERVER-002` 0.18.0.
- Supersedes/Superseded by: none. Additive: `protocol::contradicted`
  (pure), `serve::request_contradicted`, one guard arm in
  `handle_connection`, `PROTOCOL_VERSION` 29. No new variant, no new
  `ErrorCode`, no planner change.

## Purpose and scope

Since `ADR-0075` a filter no record can satisfy — `a > 5 AND a < 3`,
`a = 3 AND a > 3`, `a = 1 AND a = 2` — walked an empty range and
answered nothing, indistinguishable from a table that simply holds no
such row. The clause is a mistake in the query, not a fact about the
table, and `Malformed` is the code every other mistake in a request
already gets. This round refuses it before any read — on a connection
negotiated at 29 or above, so a client that never said 29 keeps the
empty answer it was written against.

Scope, exactly: the decision (`QCX-FR-001`); the gate (`QCX-FR-002`);
identity, proven (`QCX-FR-003`); no other surface changed (`QCX-FR-004`).

## Non-goals

- **Simplifying or rewriting the filter.** Redundant bounds stay
  (`ADR-0083` tightens them); only an empty intersection is refused.
- **Client-side detection.** The Rust and Python clients send the
  request and report the server's `Malformed`; a client-side check
  would be a second copy of the rule.
- **Contradictions across fields** (`a > b`): no such predicate exists.
- **`Ne` alone or two `Ne`**: never empty.

## Context and terminology

Read from `main` after PR #289 (protocol 28) this pass:

- **`protocol::contradicted(filter)`**: for every pair of predicates on
  one field — on any kind, two `Eq` with different literals or an `Eq`
  beside an `Ne` of the same literal; on `U32`/`I64` (one `i128`
  order), a lower bound (`Gt`/`Ge`/`Eq`) above an upper bound
  (`Lt`/`Le`/`Eq`), or equal to it with either side exclusive. Pure
  over the literals.
- **`serve::request_contradicted(&req)`**: `Query`, `Aggregate`,
  `FilteredPage`, `FilteredPageDesc`, and both sides of `Join`.
- **The gate**: one guard arm in `handle_connection`, after the version
  gates and before dispatch — `negotiated >= 29 && request_contradicted`
  → `Err { Malformed }`. Below 29, dispatch as before. A refused read
  is not counted in `dogserver_query_plans_total` (only an ok response
  records) but is in `requests_err_total` and the latency histogram.
- **Rule 3, applied to semantics**: as version 27 gated a flag bit's
  meaning, 29 gates an answer's shape — the nearest older shape below
  it is the empty answer.

## Requirements

- `QCX-FR-001` **The decision.** As "Context"; a unit matrix pins every
  empty pair, every satisfiable pair, and pairs on different fields.
- `QCX-FR-002` **The gate.** As "Context"; `PROTOCOL_VERSION` 29; the
  version tables, the pin, `SERVER-002` 0.18.0 (§4, §5, §7 item 26,
  §8, §10), the Python client's declared version.
- `QCX-FR-003` **Identity, proven.** At 29, over a real socket on
  `Memory`: `Query`, `COUNT(*)`, and the ordered page with a
  contradiction on the range field, a scanned numeric field, and a
  `Str` field are `Malformed`; satisfiable pairs (`>= 3000 AND <=
  3000`, `> 3000 AND < 5000`) still answer. At 28, over a raw socket:
  the same `Query` answers empty `Rows`. Four pre-existing socket assertions that pinned the old contract ("empty, never an error") are rewritten to `Malformed` and named: the inverted and equal-exclusive ranges in `range_query_on_the_ordered_field_matches_the_full_scan_on_memory_and_relation` (`ADR-0075`), the inverted range in `since_shaped_filtered_page_walks_the_index_and_returns_the_exact_sequence` (`ADR-0076`), the inverted-range count in `a_pure_range_count_from_the_index_matches_the_decoded_count_exactly` (`ADR-0081`), and the `= 3000 AND > 3000` case in `redundant_and_contradictory_bounds_on_the_range_field_answer_exactly` (`ADR-0083`); no other pre-existing test changed but the version pins.
- `QCX-FR-004` **Everything else unchanged.** The planner, the walks,
  every variant, the clients' request shapes.

## Considered options

- **(a) Refuse at the gate, versioned — implemented.**
- **(b) Refuse unversioned** — every client, including one at 1, sees
  `Malformed` where it saw nothing; a rule-3 violation in spirit.
- **(c) Decline.** The empty answer stays.

The owner chose this item; (a) is how it is taken.

## Proposed shape

`src/server/protocol.rs`: `contradicted`, the version, the table row,
the pin. `src/server/serve.rs`: `request_contradicted`, the guard arm.
`clients/python/…/protocol.py`, `tests/server_client_only.rs`,
`SERVER-002`: the version.

## Data/state and invariants

- `contradicted(f)` implies no record satisfies `f` (soundness); the
  converse is not claimed (a filter can be empty against a table
  without being a contradiction).

## Errors, failure, recovery, and observability

`Malformed`, at ≥ 29, before any read.

## Security, privacy, and compatibility

No new variant; every vector byte-identical; a client below 29 is
answered exactly as before.

## Acceptance criteria

1. The unit matrix.
2. Integration (`tests/server_sql_integration.rs`): `QCX-FR-003`.
3. Not measured: nothing is read.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the Python vectors suite. Independent review owed.

## Traceability

- Roadmap: `SERVER-QUERY-CONTRADICTION`.
- Decision: `ADR-0091`.
- Specification: `SERVER-001` v0.76.0 / `FR-088`; `SERVER-002` 0.18.0.
- Requirements: `QCX-FR-001`–`004`.

## Open questions

None named. The future-growth list's concrete items are all taken.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word
  ("contradiction"), as a protocol round.
- 2026-09-21: implemented as `SERVER-001` v0.76.0 / `FR-088`,
  `SERVER-002` 0.18.0, protocol 29. Acceptance criteria 1–2 are the
  tests: `protocol.rs` +1
  (`contradicted_names_every_empty_pair_and_nothing_else`) and the pin
  at 29, `tests/server_sql_integration.rs` +1
  (`a_contradictory_filter_is_malformed_at_29_and_empty_below`),
  `tests/server_client_only.rs` at 29 (lib 647, up from 646; SQL
  64, up from 63); 940 tests across 39 targets, 0 failed; the
  Python vectors suite green. Still no independent review — owed.
