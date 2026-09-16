# ADR-0068: Combining `WHERE` with `ORDER BY` — `Request::FilteredPage`

- Status: **Accepted / Implemented** (2026-09-15). See
  `docs/design/SERVER-FILTERED-PAGE-DESIGN.md` for the full design.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md`'s "Path to SQLite/DuckDB parity" item
  1 (names this exact gap: "Still absent: ... `ORDER BY` combined with
  a filter"), `ADR-0061` (`ORDER BY` itself — its own Considered
  options named this round's shape as option (b), a legitimate
  follow-on, not bundled there), `ADR-0055` (`Request::Page`, protocol
  20 — the primitive reused unchanged), `ADR-0059` (the memory-only
  `Ordered` index behind `Memory`/`Relation`'s fast `page` path, which
  carries no field data and so cannot test a predicate for free),
  `ADR-0034` (`Request::Query`, `Predicate`/`CompareOp`,
  `validate_predicate`/`predicate_matches` — the one existing predicate
  evaluator this round reuses unchanged).
- Supersedes/Superseded by: none proposed.

## Context

`docs/FUTURE-GROWTH.md`'s SQL-parity checklist names the gap directly:
*"Still absent: any query planner or optimizer, and `ORDER BY` combined
with a filter."* `ADR-0061` built `ORDER BY` itself, compiled to
`Request::Page` (protocol 20, reused with no wire change) — but refuses
at parse time the moment a query also has a `WHERE` clause
(`OrderByWithFilter`), because `Request::Page` takes no filter argument
at all.

`ADR-0061`'s own Considered options named this shape precisely as
option (b) — *"thread a `WHERE` filter through to a new, filter-bearing
`Page` variant (or an optional field on the existing one)... a real
evaluation-strategy design question this crate has never answered
(filter-then-index-walk vs. index-walk-then-filter, and what happens
when the ordered field itself is unindexed for a domain)"* — and
declined to bundle it into that round, since no consumer had asked for
it yet. Neither has one asked for it in this round; the trigger is the
same two governing documents' own "still open" language `ADR-0061`
itself was triggered by, not a new spike finding — named plainly, the
same way `ADR-0061` named its own motivation.

**A real constraint found while reading the wire contract directly,
not assumed**: `Request::Page`'s existing struct cannot gain a field
in place. `SERVER-002`'s own compatibility rule 1 — *"existing
indices, fields, and struct layouts never change"* — is absolute, and
`bincode`'s positional struct encoding means appending a field would
re-encode every already-pinned `Request/Page` vector differently. So
`ADR-0061`'s own "(or an optional field on the existing one)" phrasing
is not actually an available shape under this crate's own wire
contract; the only legal answer is a new request variant.

## Decision

**Recommended: option (a)** — a new `Request::FilteredPage { order_by,
after, limit, filter: Vec<Predicate> }`, answered `Response::Rows`
(reused unchanged), evaluated by one shared default —
`scan_all()`, filter via the existing `predicate_matches` (the same
function `Query`'s own filter and `ReplaceIf`'s guard already reuse),
then apply `Page`'s existing key-selection logic to the filtered
subset. `PROTOCOL_VERSION` 25 → 26. No adapter override needed this
round: every domain answers correctly through the one default, at the
same cost ceiling `Request::Query` already has today for the identical
filter. Named, not hidden: `Memory`/`Relation`'s `Ordered` index gives
no speed advantage to a *filtered* request this round — only the
*unfiltered* `Page` fast path keeps it, untouched by this design.

Full reasoning, the two alternatives (client-side filtering over the
existing unfiltered `Page`; decline entirely), every requirement, and
every acceptance criterion are in
`docs/design/SERVER-FILTERED-PAGE-DESIGN.md`.

## Consequences

- Positive: closes `docs/FUTURE-GROWTH.md`'s named SQL-parity gap with
  one new request variant, reusing every existing predicate-evaluation
  and page-selection primitive unchanged — no second comparator
  implementation anywhere, client or server.
- Named, not hidden: a filtered request against `Memory`/`Relation`
  loses the `Ordered` index's speed advantage this round, falling back
  to the same full-scan-then-sort cost every other domain already pays
  for `Page` — real, but no worse than what `Query` already costs
  today for the identical filter, and the *unfiltered* fast path is
  completely unaffected.
- Wire, append-only, hard-to-reverse-once-shipped: a `PROTOCOL_VERSION`
  bump and one new request/response pairing (response shape reused).
  The class of decision `WORKFLOW.md` requires design-first,
  owner-accepted before implementation — sized like `ADR-0055`/
  `ADR-0061` themselves (a bounded wire addition to an existing,
  already-accepted request family), not like `ADR-0065`'s "new
  attack-surface category" tier: no new write, no new credential, no
  new filesystem surface.

## Considered options

**(a) `Request::FilteredPage`, filter-then-order via one shared
default** — recommended; **(b)** client-side filtering over the
existing unfiltered `Page`, zero wire change, at the cost of a second,
client-side predicate evaluator and non-matching rows still crossing
the wire; **(c)** decline entirely this round, `ORDER BY` + `WHERE`
stays unavailable.

The owner's shorthand: **(a)** the new request variant, as proposed;
**(b)** client-side filtering, zero wire change; **(c)** decline.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: owner selected option (a) — `Request::FilteredPage`,
  server-side filtering, as recommended.
- 2026-09-15: implemented and merged. `PROTOCOL_VERSION` 26,
  `Request::FilteredPage` (35) answered `Response::Rows` (reused);
  `ConnectionStore::filtered_page` (default: `scan_all` → filter via
  `predicate_matches` → `page_rows`) and `validate_filtered_page`
  (`validate_page` + per-predicate `validate_predicate`, composed, no
  new checks) in `serve.rs`; `sql.rs`'s `OrderByWithFilter` exclusion
  removed, `WHERE` + `ORDER BY` now compiles to `FilteredPage`;
  `client.rs` gains `fetch_filtered_page`, `query_ordered` routes
  through it exactly when `conditions` is non-empty, the unfiltered
  path untouched. Two real corrections found during implementation,
  not deviations from the accepted decision:
  - The design's `FPG-FR-004` text ("default `Unsupported`... even
    though this round's one shared default answers every domain") was
    self-contradictory — copied from the opt-in-capability pattern
    (`backup`/`compact`) out of habit. Implemented as written in the
    design's own "Proposed shape" code sample instead: `filtered_page`
    is a trait method with a *working* default (`page`'s own
    precedent, not `Unsupported`), so a future round can still override
    it for a domain with a cheaper path, but every domain answers
    correctly today with zero overrides.
  - The design's Non-goals claimed "no Python client change,"
    reasoning from `ADR-0061`'s lack of a Python *SQL compiler*. That
    conflated two different things: `ADR-0061` never gave Python SQL
    parsing, but every wire-touching round since `ADR-0057`
    (`count_edges`) *has* given the Python reference client a raw,
    per-request method (`page`, `write_batch`, `metrics`, `backup`,
    `fetch_snapshot`, ...) — `SERVER-002`'s own §10 changelog confirms
    this every time. Corrected: `clients/python/rusty_multimodal_db/`
    gains `FilteredPage` in `protocol.py` and `Client.filtered_page` in
    `client.py`, exercised by `driver.py` and asserted in
    `tests/server_python_client.rs`, matching every prior round's
    precedent exactly.
  - The one open question left for implementation (`page_rows`/
    `page_ids` reusability over an already-filtered `Vec<PageRow>`)
    resolved with no change needed: `page_rows` already accepts a
    materialized `Vec<PageRow>` directly.
  - The other open question (an `Ordered`-index-aware override for
    `Memory`/`Relation` when the filter is a single range on the
    order-by field) was left undone, as recommended — a separable
    follow-on, not a blocker.
