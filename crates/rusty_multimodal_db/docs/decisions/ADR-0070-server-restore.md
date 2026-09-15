# ADR-0070: Server Restore Tooling — a Real `restore_backup` CLI

- Status: **Accepted, option (a)** (2026-09-15) — the owner picked
  option (a) as recommended. See
  `docs/design/SERVER-RESTORE-DESIGN.md` for the full design.
- Date: 2026-09-15
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section,
  Backup/restore bullet (names this exact gap: "Still absent: any
  `RESTORE` request or documented restore procedure"), `ADR-0065`
  (`Request::Backup`, protocol 24 — its own Decision text already
  asserts restore needs no new server code; this round tests and
  builds on that claim), `ADR-0066` (schema migration tooling — the
  exact "documented pattern plus one real, tested CLI" precedent tier
  this round follows), `ADR-0067` (`Request::FetchSnapshot` — the "no
  automated daemon shipped, an operator's own script does the rest"
  non-goal precedent this round also follows).
- Supersedes/Superseded by: none proposed. Completes (does not
  supersede) `ADR-0065`'s own left-open "restore needs no new code"
  claim with the operator-facing tool that actually exercises it.

## Context

`docs/FUTURE-GROWTH.md`'s Backup/restore bullet names the gap
directly: *"a backup is a portable, copy-safe directory ..., so
'restore' today means manually pointing a server's data directory at
the backup and restarting it, not a request or tool that does that for
you."* `ADR-0065` built `Request::Backup`/`Response::BackedUp`
(protocol 24) and its own Decision text already claims the restore
half needs no new code at all: *"the produced directory is exactly
what `open`/`open_portable` already read."* That claim has never been
exercised by a real tool — an operator today has to know it, trust it,
and perform the copy/verify/restart sequence by hand, every time.

**Why this is not a wire operation.** Every `ConnectionStore` this
crate ships is `Arc`-shared across every connection thread
`serve_tables` spawns, and every durable adapter holds an open `mmap`
over its own slot file for the process's whole lifetime. Swapping the
underlying files out from under a live `mmap` while other connections
may be reading or writing through it has no safe in-process answer —
and Windows specifically restricts overwriting a file that is actively
mapped. A `Request::Restore` could, at most, stage files for the
*next* start; the process still has to be restarted afterward for the
restore to take effect, which is the identical two-step "copy, then
restart" sequence a plain offline tool already performs, plus a new
authenticated write surface whose effect is invisible until that
restart happens.

## Decision

**Recommended: option (a)** — a new example CLI,
`examples/restore_backup.rs`, invoked `cargo run --example
restore_backup -- <backup_dir> <target_stem> <memory|entity|relation>`.
Copies every file from a `Request::Backup`-produced directory into a
fresh target directory: every file stages into one temp directory
first, and only once every file has staged successfully is each moved
individually into its real name via `std::fs::rename` (per-file
atomic, mirroring `STORAGE-014`'s own companion-file write discipline
— see the design doc's own named limitation on group atomicity),
refusing outright if any file already exists at the target (the same
"an existing target is refused, never silently overwritten" posture
`handle_backup` already has). Verifies the result by actually
domain's own `open_{memory,entity,relation}_production_stack_portable`
— a real production code path, not a byte-count proxy — and prints the
exact next step (point a server binary's `SERVER_DATA_DIR` at the
restored directory and restart it). Zero new `src/` library code, zero
new dependency, zero wire/protocol change — the identical tier
`ADR-0066`'s schema migration tooling already established and the
owner already accepted for an adjacent "offline, directory-level
maintenance operation" question.

Full reasoning, the two alternatives (a wire `Request::Restore`; declining
again), every requirement, and every acceptance criterion are in
`docs/design/SERVER-RESTORE-DESIGN.md`.

## Consequences

- Positive: closes `docs/FUTURE-GROWTH.md`'s named gap with a real,
  runnable, tested tool rather than leaving `ADR-0065`'s own "restore
  needs no new code" claim as an untested assertion — the exact
  correctness proof `ADR-0066`'s own worked migration example already
  established as this crate's bar for "a documented pattern" versus
  "a proven one."
- Positive: zero new `src/` library surface, zero new dependency, zero
  wire/protocol version bump — this round is entirely additive and
  entirely below the network layer, identical in shape to `ADR-0066`.
- Named, not hidden: this is a local, operator-run tool, not a
  networked or automated one — no scheduling, no daemon, no wire
  reachability. An operator (or their own external automation) invokes
  it by hand, once per restore, matching `ADR-0067`'s own explicit
  "no replica-refresh daemon shipped" non-goal.
- Named, not hidden: restore is a two-step "copy, then restart"
  operation, not a live one — the tool's own output says so rather
  than leaving that implicit, and Context above records exactly why a
  hot, in-process restore is not architecturally available for this
  crate's mmap-backed server model.
- This is a bounded, additive operator tool, sized like `ADR-0066`
  itself, not `ADR-0010`'s original protocol-defining tier — the class
  of decision `WORKFLOW.md` requires design-first, owner-accepted
  before implementation, which is why this is a proposal, not code.

## Considered options

**(a) A real CLI, `examples/restore_backup.rs`, as scoped above** —
recommended; **(b)** a `Request::Restore { name }` wire operation,
gated like `Backup`/`Compact` — cannot actually take effect without a
subsequent process restart either, so it gains nothing over (a) except
wire reachability, at the cost of a new authenticated write surface
whose effect is invisible until an unrelated future restart and a
`PROTOCOL_VERSION`/`SERVER-002` bump; **(c)** decline again, the
manual copy-and-restart procedure stays the whole story, `ADR-0065`'s
own "restore needs no new code" claim stays untested.

The owner's shorthand: **(a)** the CLI, as proposed; **(b)** a wire
`Request::Restore` instead; **(c)** decline.

## Acceptance and implementation

- 2026-09-15: proposed, design only.
- 2026-09-15: the owner picked option (a) — the `restore_backup` CLI,
  as recommended. Implementation delegated to Codex (`codex-build`),
  independently inspected by Claude before merge.
