# ADR-0012: One task store: rusty_tick holds tasks, Mill Board is the agent view

Status: Accepted
Date: 2026-10-26

## Context

Three surfaces hold tasks today, and they drift:

- **rusty_tick** (`crates/apps/rusty_tick`): a TickTick-style task app with a
  web UI, a JSON API and an MCP server. It is where a person already reads and
  edits tasks.
- **rusty_mill_board** (`crates/apps/rusty_mill_board`): the agent kanban. It
  enforces who may move a card where (capabilities, claims, projects), feeds
  the runner and rusty_orch, syncs GitHub issues, and keeps its own
  `rusty_multimodal_db_engine` store behind a `Store` port (`src/store.rs`).
- **The Mill Board artifact**: 58 planning cards (summary, Claude Code prompt,
  notes, deps) in a claude.ai artifact database. It is the board the owner
  actually works from today.

remind-me holds memory, and was at risk of growing into a fourth task store.

The owner decided on 2026-10-26: tasks live only in rusty_tick, Mill Board
becomes a view over rusty_tick tasks tagged for agent work, and remind-me keeps
the why behind a task and links to it by id, never a copy.

## Decision

### 1. Mill Board keeps its rules and surface, and swaps its store

Mill Board's domain (transition table, capabilities, claims, projects), its
MCP and web API, the runner, rusty_orch's client and the GitHub sync stay as
they are. A new adapter, `TickStore`, implements the existing `Store` port
over rusty_tick's HTTP API (`--tick-url`, token in `RUSTY_TICK_TOKEN`).
Nothing that talks to the board changes.

This is the smallest change that makes rusty_tick the source of truth: one
adapter beside the two that exist, behind a port the board already has. The
mmdb adapter is removed once the tick-backed board has run in place of it;
the in-memory adapter stays for tests.

### 2. Field mapping: board card to rusty_tick task

No field is added to rusty_tick's `Task`, so rusty_tick needs no data
migration. Everything the transition rules read (column, claim, project) is
on the task itself, so one `If-Match` write covers it, the same
compare-and-swap the board has today. What does not fit on the task lives in
one `board` document per task, with its own compare-and-swap (below).

| Mill Board `Card` | rusty_tick | Notes |
| --- | --- | --- |
| `id` | `Task.id` | same UUID |
| `rev` | `Task.version` | written with `If-Match`; a stale write is the board's `Conflict` |
| `title` | `title` | |
| `body` | `notes` | |
| `column` | tag `board:<column>` | `Done` also sets status `done`. Same label scheme the GitHub sync already uses |
| `claim` | tag `claim:<actor>` | tags are case-insensitive, so actor names must be lowercase (validated at the identities boundary) |
| `number` | tag `number:<n>` | board-wide, on the task so it lands in the creating `POST`. Each number is first reserved as a `board_number` doc (id derived from the number, `If-None-Match: *`), never released: unique across boards sharing the store and across untag, trash and restore. A lookup by number that finds two cards (a tag copied by hand) is refused |
| `tags` | `tags` | `agent`, `board:`, `claim:` and `number:` are reserved; `card_update` refuses them. rusty_tick keeps tags lowercase, so they read back lowercase |
| `project` | `listId` | the list's name is the project, and a project's list is created on first use. The Inbox is the `default` project |
| `comments` | tick `comment` docs | existing shape `{v, taskId, text, createdMs}` plus `by`, so the tick UI shows who wrote it |
| `issue`, `synced_text`, `execution` | tick `board` doc, id = task id | written with its own `If-Match` (below) |
| `created_ms`, `updated_ms` | same | |
| *(new)* order | `priority`, then `sortOrder` | see 4 |
| *(new)* `deps` | `board` doc | see 4 |

A task is on the board when it carries the tag `agent`. That tag is the
owner's switch: tag a task `agent` in rusty_tick (phone, web, MCP) and it
appears in Backlog; untag it and it leaves the board. `board:` and `claim:`
tags are the board's state and are written by it.

**The `board` document's compare-and-swap.** `bind_execution` and
`recover_execution` are compare-and-sets today: the first refuses a second
logical execution, the second refuses a stale generation, and both rely on the
card's revision. They change only `execution`, never the column or claim, so
under `TickStore` each one is a single write to the `board` document. That
write must be fenced by the task as well as the document: `Board::apply`
authorizes it from the task's state (its project, for the actor's scope), so a
task reassigned between the read and the write must refuse it, as the
whole-card revision does today.

1. `GET /tasks/{task id}` (version `v`) and `GET /docs/board/{task id}` (ETag
   `e`, or none).
2. The board checks scope and runs the domain function on the card assembled
   from both, exactly as now.
3. `PUT /docs/board/{task id}` with `If-Match: <e>` (or `If-None-Match: *`)
   **and** `If-Task-Match: <v>`. rusty_tick checks both under its request
   lock, the same lock every task write takes, so neither the document nor
   the task can change between the check and the write. Either mismatch is a
   412, the board's `Conflict`, and the caller retries from 1 as it does
   today.

**Commit order, as built.** Every board write is one request fenced on the
task version the board read, and every write moves that version: a task `PATCH`
under `If-Match` (an empty one still moves it), or a `board` document or
comment `PUT` under `If-Task-Match`, which rusty_tick checks and then moves the
task's version in the same locked step (task first, so a crash between leaves a
moved version and the old document). The version therefore names the task and
its board document together, the card's revision is `version - 1`, and a write
from any read older than the latest write is refused. Reads take the task
before its document, so a read racing a write sees a newer document under an
older version, and the write it leads to is refused. A comment is an explicit
append, so a comment the owner adds meanwhile (which does not move the
version) is kept. Only `card_plan` with both a priority and dependencies needs
two requests, each fenced; if the second fails, the caller sees an error with
the priority applied, and the same plan again completes it.

A document is one engine record, so a crash leaves the old document or the new
one, never half of either. Two boards (or a board and a stray client) cannot
both bind, and a write authorized against a task that has since moved project
is refused. `issue`, `synced_text` and `deps` are written the same way. No
board write changes both the task and the document, so no write needs the two
to commit together; the task version only fences the document write.

### 3. rusty_tick changes: three, all small

1. A `board` document kind (`docs.rs` `KINDS` and the web `DocKind`).
   Purging a task deletes its `board` document, as it already deletes its
   comments (`Service::purge_task`).
2. One new route, `GET /docs/{kind}/{id}` (today only the whole kind can be
   read), returning the document with an ETag. Document writes take
   `If-Match`/`If-None-Match: *`, as task writes already do. The ETag is the
   document's `updated_ms`, kept strictly increasing per document, so `Doc`
   needs no new field. A `board` document's id must be a live task's id, and
   its writes also require `If-Task-Match: <task version>`, checked under the
   same request lock, and move the task's version (as built; a comment may be
   written the same way).
3. Comments render an optional `by` (`features/tasks/detail/Comments.tsx`).

One new route, no `Task` field, no schema bump.

### 4. Picking the next agent task

`card_take` keeps its rules: queues nearest Done first (Review, QA, Building,
Ready), only columns the caller's capabilities can move forward, only its
projects, never a card someone else holds. Two changes:

- **Order within a queue** is tick priority (high first), then the manual
  order the owner sets by dragging in rusty_tick, then age. Today it is age
  only. The card gains a `priority`; the memory store fills it from `0`.
- **Dependencies:** a card whose `deps` are not all Done is skipped. The
  artifact board uses deps on 21 of 58 cards; the crate board has none.

A card imported from a GitHub issue gets an id derived from the issue number,
so an import retried after a crash finds its task instead of making a second,
and the sync relinks a card whose link was lost before it would export it as a
new issue. The id ignores the repository, so a board syncs one repository.

Taking a card claims it in the same write, as today: `board:building` and
`claim:<actor>` land together under `If-Match`, so two agents cannot take the
same card.

### 5. Status flows back through the same task

Every board write is a write to the tick task, so the owner sees an agent's
move, claim and comments in rusty_tick at once, and an edit in rusty_tick is
what the board reads next. There is no sync loop and no second copy.

### 6. remind-me links to a task, never copies it

- **Format:** `tick:<task uuid>`, written in a memory's content. remind-me's
  reference detector (`remind_me_core/src/extract.rs`) learns one rule that
  stores it as a `memory_references` row of a new kind `task` with the UUID as
  value. The rule matches only a whole token that is `tick:` followed by a
  UUID, so a list reference (`tick:list:<list uuid>`, defined by remind-me's
  ADR-0027) is never filed as a task link. `remind_me_references` then
  answers "everything about this task".
- **When:** a decision or rationale about a task is saved with
  `remind_me_add`, and a session that worked a card ends with an
  `auto_capture` naming the card's `tick:` id. The task never stores memory
  text; at most a comment points back to a memory id.
- remind-me's own `remind_at` reminders stay: they resurface a memory, they
  are not tasks.

### 7. Migration of the artifact cards, with rollback

A one-shot script (not shipped in a crate) reads the artifact database export
and writes rusty_tick through its API:

| Artifact card | rusty_tick |
| --- | --- |
| `title` | `title` |
| `summary`, `prompt`, `notes` | `notes` as three Markdown sections (`## Summary`, `## Prompt`, `## Notes`) |
| `status` backlog, next, doing, waiting, done | `board:backlog`, `board:ready`, `board:building`, `board:blocked`, `board:done` + status `done` |
| `kind` | tag `kind:<kind>` |
| `track` | tag `track:<track>` |
| `tags` | tags |
| `order` | `sortOrder` |
| `deps` (card numbers) | `board` doc `deps` (task UUIDs), in a second pass once every card has an id |
| `n` | last line of the notes: `Migrated from Mill Board card #n` |
| *(all)* | tags `agent` and `migrated:mill-board`, list `rusty_mill` |

The marker (the tag plus the notes line) travels in the same create request as
the task, so a task never exists without it.

Steps:

1. Export the artifact database to JSON (read only; the artifact is not
   changed).
2. Dry run: print the 58 tasks the script would create and the deps it
   resolved. The owner checks it.
3. Run it. Each run first rebuilds the map from card number to task id from
   rusty_tick itself: every task tagged `migrated:mill-board`, trash
   included, keyed by its notes line. Pass one creates only the cards missing
   from that map. Pass two writes each task's `board` document with its full
   `deps`, which is the same write however often it runs. A crash at any point
   leaves either a marked task or none, so running again finishes the job
   without a duplicate. Two marked tasks for one card number stop the script
   before it writes anything.
4. Mark the artifact read-only (a banner pointing at rusty_tick). From here,
   rusty_tick is the only place to edit.

**Rollback:** the artifact is never written by the migration, so it is still
whole. The script moves every task tagged `migrated:mill-board` to the trash
(`DELETE /tasks/{id}?leaf=true` under `If-Match`); remove the banner, and the
artifact is the board again. Rollback is reversible and never deletes anything
for good: rusty_tick's restore brings a task back. Trashing a task takes its
subtasks with it, so `leaf=true` makes rusty_tick refuse (409), in the same
step as the write, a task that has any subtask; the migration creates none, so
anything the owner added under a migrated task stays where it is. The script
checks for such subtasks first, so a refusal normally writes nothing, and it
refuses a rusty_tick without `leaf`. The trashed tasks keep their markers, so
a later run counts them as migrated and creates nothing: to migrate again,
restore them (the run then brings their documents up to date) or delete those
tasks for good from the trash, by hand, first. A deployed crate board `--store`, if any, is
exported with `board_list` and imported the same way; its store file is kept
until the tick-backed board has run.

## Consequences

- **The board's event log is no longer durable or atomic with the card.**
  rusty_tick has no change log, so `TickStore` cannot commit a card and its
  event together. Events become an in-process log: the board's own writes,
  plus an `edited` event (actor `tick`) when a task's version moves without a
  board write, found when the event cursor is polled. The web UI uses events
  only to know when to re-list, which this keeps; the runner and rusty_orch
  do not read events. A restart starts a new log, and a watcher re-tails.
- **The owner's edits in rusty_tick are not checked against the transition
  table.** Moving a tag in rusty_tick moves the card. The owner holds every
  capability, so this is the owner acting, not a hole; agents still go
  through the board, which still checks.
- **The board needs rusty_tick up.** If it is down, the board's `/health`
  reports 503 and writes fail; nothing is lost.
- **Comment author** is shown in rusty_tick only after change 3.3.
- rusty_tick's tag budget (20) holds: `agent`, `board:`, `claim:`, `kind:`,
  `track:` and, on migrated tasks, `migrated:mill-board` leave 14.

## Alternatives considered

- **Host the board inside the rusty_tick process**, sharing its engine, so a
  card and its event commit together. Keeps the durable log, but couples two
  crates' processes and stores for a log nothing outside the board UI reads.
  Revisit if an event consumer needs durability.
- **Add board fields to `Task`** (column, claim, deps, prompt). One write for
  everything, but a `Task` schema bump and data migration in rusty_tick for
  fields only the board reads. Tags and one document kind cover them.
- **Client-chosen task ids** (a UUIDv5 of the card number; `CreateTask`
  already accepts an `id`) would also make the migration idempotent. The
  marker tag is kept because it does a second job the ids cannot: it selects
  every migrated task for rollback with one query, and shows in rusty_tick
  where a task came from.
- **Keep the board's store and sync it with rusty_tick.** Two copies and a
  reconcile loop: the drift this decision exists to remove.

## Implementation, in order, each its own PR

1. rusty_tick: `board` doc kind (purged with its task), `GET
   /docs/{kind}/{id}`, document `If-Match` and `If-Task-Match`, comment `by`.
2. rusty_mill_board: `priority` and `deps` on the card, `card_take` order and
   dep skip, reserved tags (works on the memory store, tested there).
3. rusty_mill_board: `TickStore` adapter, `--tick-url`, in-process events.
4. remind-me: `task` reference kind and the `tick:` detector rule.
5. Migration script, dry run, then the run (needs the owner's go-ahead).
6. Remove the mmdb adapter after the tick-backed board has run.
