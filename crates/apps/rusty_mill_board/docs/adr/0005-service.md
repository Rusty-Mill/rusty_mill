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
- **Clean stop.** SIGTERM and SIGINT call the server's `ShutdownHandle`; the accept loop ends and the
  process exits 0. Nothing needs flushing, since every commit is one durable record; the point is to
  stop answering rather than die mid-request. Tokio's `signal` feature is used (already in the
  workspace graph); no new crate.
- **Backups run inside the process** (`--backup-dir`, `--backup-secs`, `--backup-keep`), not from a
  timer that copies files. The store copies its three files while holding its commit lock, so the
  copy is one moment of the board; an external `cp` of a live file has no such guarantee, and stopping
  the service for it would drop writes. A backup is built under a temporary name and renamed, so an
  interrupted run never looks complete. The settings files travel with it, owner-only. Old backups are
  pruned by name (zero-padded Unix ms sorts by time).
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
