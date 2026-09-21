# ADR-0083: Query Planner Step Eight — Bound Tightening

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0082` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "bound tightening" open question every planner round since
  `ADR-0075` has carried; the fork below is held open for the owner at
  review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-BOUND-TIGHTENING-DESIGN.md`
  (the full design), `ADR-0075` (whose `QPR-FR-003` took the first
  bound per side in wire order and filed tightening under the cost
  model), `ADR-0081`/`ADR-0082` (whose walk-only paths refused a second
  bound per side — the limit this round removes), `ADR-0076`
  (`bounded_walk_start`, already the tightest lower bound for the page
  walk), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`:
  `tightest_bounds`, used by `plan_query`, `counted_walk`, and
  `keyed_walk`; `counted_walk_applies` widened. No generic, adapter,
  wire, protocol, client, or file-format change.

## Context

Since `ADR-0075` a filter with two lower bounds on the range field
walked from the *first* in wire order and dropped the rest over decoded
rows: `updated_at >= 40000 AND updated_at >= 50000` read 10,000 records
it then rejected. `ADR-0081` and `ADR-0082` declined a second bound per
side outright so that their walks stayed exact. "Tightening" had been
filed under the cost model since `ADR-0075`; read again, it is not one.
Which of two bounds on one side is tighter is decided by comparing
their literals to each other, never to the table, and the tightest
implies every other on its side — so the walk between the two tightest
admits exactly the records every bound admits. `bounded_walk_start`
(`ADR-0076`) already did this for the page walk's lower side.

## Decision

Implement: `tightest_bounds(range_field, filter) -> (Option<usize>,
Option<usize>)` — the positions of the greatest lower literal (an
exclusive `Gt` tighter than an inclusive `Ge`/`Eq` at a tie) and the
least upper literal (`Lt` tighter than `Le`/`Eq`), literals compared as
`i128`, other fields and `Ne` ignored, pure. `plan_query`'s
`IndexRange`/`IndexIntersect` carry those positions; `counted_walk_applies`
accepts any number of `Gt`/`Ge`/`Lt`/`Le`/`Eq` on the range field (still
nothing else); `counted_walk` and `keyed_walk` walk between the tightest
two. A contradiction (`a = 3 AND a > 3`) walks an empty range and
answers nothing, as `ADR-0075`'s guarded `range_by` already did for an
inverted pair. Every result is the full scan's, proven on the fixture,
by `tightest_bounds`' unit matrix, and over a socket on `Memory` and
`Reminder`.

The fork, held for the owner:

- **(a) As implemented.** Tighten at the planner.
- **(b) (a) plus rejecting a contradiction at validation** — `Malformed`
  before any walk; a wire-visible change (an empty answer becomes an
  error), so a protocol round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the 1% range with a redundant looser lower bound ahead
  of it (`range-tight`): `query` 12,231.3 → 1,369.8 µs,
  `count(*)` 13,136.5 → 58.7, `fpage-50` 130.2 →
  114.9; a redundant bound no longer widens the walk, and a
  pure-range aggregate with two bounds on a side no longer decodes.
- Named, not hidden: the redundant predicate stays in the filter and is
  still re-checked on the decode paths; a contradiction is an empty
  answer, not an error; still no estimate of anything.
- Named, not hidden: two pre-existing assertions are rewritten to the
  new rule (the "first in wire order" plan assertion of `ADR-0075`'s
  test; the two "ineligible" shapes of `ADR-0081`'s eligibility test) —
  the first rewrite of a planner-round assertion in this line, and the
  reason it is called out.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.68.0 /
  `FR-080`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 635 (up from 633), `server_sql_integration`
  58 (up from 57), every other target unchanged and green, 920
  tests across 39 targets, 0 failed; two pre-existing assertions
  rewritten as named above, no other pre-existing test changed. Measured
  (`benches/server.rs`'s new `memory-planner` `range-tight` row —
  `50000 <= updated_at_unix_ms < 51000` with a redundant
  `updated_at_unix_ms >= 40000` ahead of it, 100K records over a real
  loopback socket, an idle 4-core Linux container, added and measured on
  the pre-change code first): `query` 12,231.3 → 1,369.8 µs,
  `count(*)` 13,136.5 → 58.7, `fpage-50` 130.2 →
  114.9 — `RESULTS.md`. The fork above remains the owner's at
  review; (a) is what merges if the PR merges unchanged.
