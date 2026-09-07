# Server Durable Data Directory Design (Proposed)

- Status: **Proposed** (2026-09-07) — implemented on the same branch as
  `SERVER-001` v0.43.0 / FR-053, the `ADR-0046`–`ADR-0052` cadence, for
  the owner to accept as designed or send back. No wire change.
- Date: 2026-09-07
- Related: `ADR-0048`/`docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`
  (`memory_server`, which this round makes restartable), `ADR-0046`
  through `ADR-0052` (the runtime writes whose survival across a
  restart this round is about), `ADR-0025` (`SERVER_TXN_JOURNAL_PATH`,
  the precedent for a path-valued operational variable), `ADR-0032`
  (`ServeOptions`, the operator-facing surface), `STORAGE-015`/
  `SYMPORT` (the portable reopen this round builds on).
- Supersedes/Superseded by: none. Adds one environment variable to one
  binary and one helper per domain; changes no file format and no
  `Request`/`Response`.

## Purpose and scope

Seventeen rounds gave a running store every write the consumer needs
and made each survive a *reopen* — every test of that survival calls
`open_*_production_stack_portable` in-process. The one binary the
consumer would run, `memory_server`, never does: it seeds the sample
dataset into a fresh per-process scratch directory on every start
(`std::env::temp_dir()/memory_server_<pid>`), so nothing written to it
outlives the process. An integration spike against the consumer found
this on the first restart. A backend that forgets on restart is not a
backend; this round closes that.

Scope, exactly: `open_or_create_memory_production_stack` and
`open_or_create_entity_production_stack` — open from the files when
the slot file exists, create empty when not (`DDR-FR-001`);
`SERVER_DATA_DIR` on `memory_server` — set, the two tables live there
durably; unset, exactly today's behaviour (`DDR-FR-002`); the startup
line names which (`DDR-FR-003`).

## Non-goals

- **A caller-supplied dataset in the durable directory** — no seeding,
  no import. The consumer populates its tables through the wire; an
  import path is a client's (the Python client's) to script.
- **`reminder_server`, `entity_server`, `dog_server`** — the same
  variable belongs on each; this round changes the consumer's binary
  only and names the mirror as an open question, so the diff stays one
  decision.
- **Repairing a partial directory** — a slot file without its
  companion, or the reverse, is reported at startup by the reopen path
  (`RecordBlobUnreadable` and friends), never silently recreated.
  Deleting files is the operator's act, not this binary's.
- **A lock against two processes on one directory** — the multiprocess
  story (`STORAGE-014`) is unchanged; nothing here adds to or subtracts
  from it.
- **Journal placement** — `SERVER_TXN_JOURNAL_PATH` stays its own
  variable; a default of `<dir>/journal` would silently change a
  configured deployment's crash story and is declined.

## Context and terminology

- **Portable reopen**: `open_*_production_stack_portable(path)` rebuilds
  a stack from `path`, `<path>.records`, the insert log, every edge
  blob, edge log, and the label manifest — no record list, no edge
  list. It is what every survival test uses, and what a restart must.
- **Caller-list open**: `open_*_production_stack(records, edges, path)`
  reconciles by id against a list the caller supplies; runtime edges
  the list lacks are rewritten away (documented since `ADR-0047`). A
  served process has no such list, so it must never use this form.
- **Slot file**: the mmap file at `path` itself; its presence is the
  create-or-open decision (`DDR-FR-001`).

## Requirements

- `DDR-FR-001` **Open-or-create helpers.** `crate::generic::memory::
  open_or_create_memory_production_stack(path)` and `crate::generic::
  entity::open_or_create_entity_production_stack(path)`: if `path`
  exists, `open_*_production_stack_portable(path)`; else
  `create_*_production_stack` with no records and no edges. Errors are
  the two constructors' own. An empty stack accepts every runtime
  write, and a second call reopens what the first left — inserts,
  links, deletes, and a compaction included.
- `DDR-FR-002` **`SERVER_DATA_DIR`.** On `memory_server`: set, the
  directory is created if missing and `<dir>/memories.mmap` /
  `<dir>/entities.mmap` are each open-or-created; unset, the sample
  dataset in a per-process scratch directory, byte-for-byte today's
  behaviour. Any failure is a startup error naming the file.
- `DDR-FR-003` **The startup line** reports `data: durable at <dir>` or
  `data: sample data, scratch at <dir> (NOT durable)`, beside the other
  knobs.

## Considered options

- **(a) One environment variable selecting a durable directory,
  open-or-create per table, empty on first start — proposed.** The
  smallest change that makes the binary a backend; matches how every
  other operational knob is set; the reopen path is the one every
  survival test already trusts.
- **(b) A positional argument.** The binary's one positional is the
  listen address; a second is a parse convention this crate has avoided
  (no argument-parsing dependency).
- **(c) Durable by default, at a fixed path.** Changes what every
  existing invocation does and picks a path for the operator.
- **(d) Seed the samples into the durable directory on first start.**
  Puts three fake memories into a real backend.
- **(e) Decline** — leave persistence to an out-of-tree binary.

## Proposed shape

`src/generic/memory.rs` (`open_or_create_memory_production_stack`),
`src/generic/entity.rs` (`open_or_create_entity_production_stack`),
`src/bin/memory_server.rs` (`DataLocation`, `open_stores`, the variable,
the startup line). No `src/server/**`, protocol, client, or Python
change.

## Data/state and invariants

- With `SERVER_DATA_DIR` set: after any sequence of runtime writes and
  a restart, every read answers as it did before the restart — the
  invariant every `*_portable` survival test already states, now for
  the process.
- The first start in an empty directory serves two empty tables; the
  scratch default still serves the three sample memories and three
  entities.

## Errors, failure, recovery, and observability

An unwritable directory, an unreadable slot file, or a missing
companion is a startup panic naming the path — the binary's existing
convention for every misconfigured variable. Nothing is recreated on
error. The startup line is the observability.

## Security, privacy, and compatibility

A path from the environment, like the journal and log paths; the same
trust model. No wire change, no format change; a directory this
binary wrote is exactly what `create` plus runtime writes produce, so
any build since `ADR-0051` reads it.

## Acceptance criteria

1. Store: for `Memory`, the first `open_or_create` at a path is empty;
   two inserts and a link survive a second call; a delete and a
   compaction survive a third. For `Entity`, the same with three
   inserts and a link, the name index included.
2. Binary, over a real socket: started with `SERVER_DATA_DIR` on an
   empty directory, `get` of a fresh id is `None`; a memory and an
   entity inserted and linked; the process killed; restarted on the
   same directory, the memory reads back with its content, the entity
   is present, `ListTables` unchanged. Started without the variable,
   the startup line reports scratch and the samples are served.
3. `cargo fmt --check`, clippy `-D warnings` on the three feature sets,
   every test, the Python vectors, rustdoc at the baseline.

## Verification plan

Criterion 1 is `cargo test --all-features` (two unit tests). Criterion
2 was run by hand for this round against the built binary and the
Python client and is recorded in `docs/PROJECT-STATUS.md`; an automated
binary-level harness is an open question below.

## Traceability

- Roadmap: `SERVER-DATA-DIR-DESIGN`, `SERVER-DATA-DIR`. Decision:
  `ADR-0053`.
- Specification: `SERVER-001` v0.43.0 / `FR-053`. No `SERVER-002`
  change.
- Requirements: `DDR-FR-001`–`003`.

## Open questions

- **The other binaries** — `reminder_server`, `entity_server`,
  `dog_server` should take the same variable; one round each, or one
  round for all three.
- **A binary-level test** — no integration test spawns a binary
  (`CARGO_BIN_EXE_*` is unused); criterion 2 is manual until one does.
- **A conditional replace** — the consumer's hub does last-writer-wins
  on `updated_at`; `Request::Replace` is unconditional. Named by the
  same spike; the strongest candidate for the next wire round.
- **A compaction policy**, **unlinking one edge**, **entity merge as a
  request** — unchanged from `ADR-0052`'s open questions.

## Change history

- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The eighteenth round in the `rusty_remind_me`-motivated
  line; found by an integration spike's first restart.
