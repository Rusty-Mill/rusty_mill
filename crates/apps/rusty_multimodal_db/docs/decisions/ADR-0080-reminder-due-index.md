# ADR-0080: `Reminder` Under `Ordered` — a Sorted Index on `due_at_unix_ms`

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0079` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "`Reminder` under `Ordered`" open question `ADR-0075` carried and
  the `docs/FUTURE-GROWTH.md` gap "a range on `Reminder::due_at_unix_ms`
  (an equality index, not an `Ordered` one)"; the fork below is held
  open for the owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-REMINDER-DUE-INDEX-DESIGN.md` (the full
  design), `ADR-0059` (`Ordered`, whose Non-goals named this as "a
  one-line stack change and an adapter `page` override, when a
  consumer asks"), `ADR-0036` (`Reminder`, whose `RMD-FR-003` sent
  `due_at < now` — "the actually common case" — through a full scan),
  `ADR-0075`–`ADR-0079` (every planner round `Reminder` now reaches
  through `range_field`), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `DueAtOrder`,
  `OrderedField` for `Reminder`, `ReminderCore`, the wrapped
  `ReminderProductionStack`, a portable-reopen constructor, five
  adapter overrides; no file-format, wire, protocol, or client change.

## Context

`Reminder` is the consumer's own domain, and its spec named its common
query at birth — `due_at < now` — as a full scan, because `due_at` got
an equality index and the sorted index did not exist yet. Five rounds
later `Memory` and `Relation` have a range path, an O(page) walk, a
walk past rejects, an index intersection, and a budget on it — each
reached through one adapter declaration, `range_field` — and `Reminder`
has none of them. `ADR-0059` said what it would take: a wrap and a
`page` override. Measured on a 100K table before this round, the
due-now page cost 53,278.9 µs, a due count 52,913.6, a narrow due
window 56,733.3 — full scans all.

## Decision

Implement: `ReminderProductionStack = Ordered<ReminderCore, Reminder,
DueAtOrder>` with the three constructors wrapping in `Ordered::new` and
`open_reminder_production_stack_portable` new; the adapter declares
`range_field = due_at_unix_ms`, walks `range_ids`/`range_ids_limited`
over the index, and overrides `page`/`filtered_page` exactly as
`Memory`'s does. `due_at`'s equality index stays. Every request returns
the identical set or sequence, proven at the generic, adapter, and
socket levels.

The fork, held for the owner:

- **(a) As implemented.** The wrap; the equality index kept.
- **(b) (a) plus retiring `DueAtField`'s `HashMap`** — `filter_eq` on
  `due_at` as a one-key range walk; less memory at open, a
  type-parameter change and a measurement. A second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the due-now page 53,278.9 → 121.9 µs, the due
  count 52,913.6 → 22,312.4, the due window 56,733.3 →
  476.6; `docs/FUTURE-GROWTH.md` drops "a range on
  `Reminder::due_at_unix_ms`"; the consumer's `rusty_remind_me` due
  listing takes the walk with no client change.
- Named, not hidden: one decode per record at every open to rebuild
  the index (`ADR-0059`'s ~81 ms per 100K), now paid by `Reminder` too.
- Named, not hidden: two indexes over one field (`HashMap` and
  `BTreeSet`) until option (b).
- Named, not hidden: the two portable reopens in existing tests are
  renamed to the new constructor; no other pre-existing test changed.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.65.0 /
  `FR-077`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 627 (up from 625), `server_sql_integration`
  55 (up from 54), every other target unchanged and green, 909
  tests across 39 targets, 0 failed. Measured (`benches/server.rs`'s
  new `reminder-due` rows, 100K `Reminder` records over a real
  loopback socket, an idle 4-core Linux container, added and measured
  on the pre-change code first): the due-now page 53,278.9 →
  121.9 µs, the due count 52,913.6 → 22,312.4, the due window
  56,733.3 → 476.6 — `RESULTS.md`. The fork above remains the
  owner's at review; (a) is what merges if the PR merges unchanged.
