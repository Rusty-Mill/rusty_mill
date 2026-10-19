# ADR-0005: Running the board as a service

- **Status:** Accepted (2026-10-19)

## Context
The board is meant to stand all the time and be reached by several models. As shipped it was a
foreground process: SIGTERM killed it mid-request, a second process on the same file would diverge
from the first (state lives in memory, rebuilt from the file at open), nothing said whether it could
still commit, a copy of the live file was not guaranteed consistent, and the first admin's token
went to stderr, which a service keeps in its journal. `rusty_remind_me` already runs this way
(system units, `Restart=always`, loopback, owner-only token files), so the board copies that shape.

## Decision
- **One owner per store.** `MmdbStore::open` takes `<path>.lock` (the engine's `DirLock`, an advisory
  file lock released when the process ends) and refuses with "in use by another process". No daemon
  is needed: the board process is the only client of its store; models use HTTP.
- **`GET /health`**, unauthenticated and open to any `Host`, says only whether the store can still
  commit: 503 once an append of unknown outcome has left it refusing writes. Recovery stays a restart
  (the reopen replays the log); systemd's `Restart=always` plus this probe is the contract, and the
  process does not exit on its own.
- **Clean stop.** SIGTERM and SIGINT stop the accept loop (`rusty_serve`'s `ShutdownHandle`; no new
  crate, tokio's `signal` feature is already in the workspace graph). Then the process waits, to a
  10 second deadline, for the requests inside their handlers and for a backup under way, and exits 0.
  `InFlight` counts them; once a stop has closed it, a request is answered 503 without running, so
  nothing starts that the stop cannot wait for. A commit whose reply is cut off is durable but looks
  failed to its caller, who may then repeat a `card_add`: that is what the wait prevents. Idle
  keep-alive connections and long streams are not waited for; the deadline and the exit end them.
  `rusty_serve` still detaches its connection threads (it predates this change); the wait is at
  the handler, which is where the board's work happens.
- **Backups run inside the process** (`--backup-dir`, `--backup-secs`, `--backup-keep`), not from a
  timer that copies files. The store copies its files while holding its commit lock, so the copy is one
  moment of the board; an external `cp` of a live file has no such guarantee, and stopping the service
  for it would drop writes. Layout: `board-<unix ms>/store/` (the store's files under their own
  names) and `board-<unix ms>/settings/` (`identities.json`, `projects.json`, `roles.json`,
  `github.json`, whatever they are called on the host), so settings that share a file name, or
  share one with the store, cannot overwrite one another; restoring puts each back where its flag
  points.
- **Settings are copied as a set.** `Settings::snapshot` holds the lock every settings change holds,
  copies the files, and loads the copy as a restore would (roles, projects, then the agents against
  them, then the GitHub sync); a set that does not load fails the backup. Otherwise an agent could
  be copied with a role that the copy of the roles, taken after an admin changed both, lacks. The store
  is copied first and the settings after: a project with cards cannot be removed, so settings read
  later still name every project the copied cards belong to. The settings lock and the store's commit
  lock are never held together; the order is settings lock first, and no code waits for it while holding
  the commit lock.
- **Publication is durable before anything is retired.** A backup is built under `.board-<ms>`, its
  directories are fsynced, it is renamed to `board-<ms>`, the backup root is fsynced, and only
  then are older backups pruned. A run that died leaves a `.board-*` directory, which the next run
  removes. On Unix the directory sync is an fsync; elsewhere `std` cannot flush a directory and the
  guarantee is the filesystem's own journalling of the rename.
- **No backup, rotation or compaction after an append of unknown outcome.** The durable log may then
  hold an event the in-memory state lacks. Compaction rewrites the records from memory and clears the
  log, which would erase the one event a reopen recovers, and a rotation could retire the last copies from
  before the failure. `MmdbStore::backup` and `compact` refuse under the same lock as commit, and
  `Backups::run` stops at `health()` before it creates or deletes anything. A restart settles it.
- **Compaction after each backup**, under the same lock, after the copy (so the backup never depends
  on it). Every commit appends to `<path>.inserts`; open replays it, so without compaction the open
  time grows with the board's whole history.
- **The first admin's token is not logged.** On a terminal it is printed once, as before. Otherwise it
  is written to `first-admin.token` (owner-only) beside `identities.json`, and only the path is
  logged.
- **Deploy files live in the crate** (`deploy/`): the unit, an env example, and a README with install,
  restore and rollback.

## Consequences
- A service stop, restart or crash loses no committed card; a backup is a restorable board.
- Backups share the disk of the store. Copying `backups/` off the host is the operator's job.
- No TLS: the server speaks plain HTTP, so anything beyond loopback needs a TLS proxy in front.
- The store's lock is advisory: it stops another copy of this program, not an arbitrary writer.
- Windows: `SIGTERM` is not handled there (Ctrl+C is); the unit is Linux-only.
