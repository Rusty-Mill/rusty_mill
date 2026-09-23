# ADR-0118: `replica_refresh`, the Operator Recipe as Running Code

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1-5", item 4 — replication beyond a cold
  standby). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0067` / `docs/design/SERVER-REPLICATION-DESIGN.md`
  (`FetchSnapshot`, and the six-step operator recipe it named and did
  not ship), `ADR-0070` (`restore_backup`, the CLI shape and the
  staged-then-renamed discipline), `ADR-0065` (`Backup`, the file set),
  `ADR-0092` (directory syncs).
- Supersedes/Superseded by: none. Additive: `examples/replica_refresh.rs`,
  `examples/support/replica_refresh_lib.rs`, `tests/replica_refresh.rs`.

## Context

Replication is a cold standby: `FetchSnapshot` streams a table's files
to a `Replication`-class client, and "an operator's own script fetches,
writes to local disk, and restarts a second `memory_server`". The
growth document lists what is absent — incremental shipping, a refresh
daemon, failover, write forwarding, membership. Of those, one is
bounded and fits the crate's existing shape: the script. Incremental
shipping stays out because the journal is unsuited to tailing by
design (`ADR-0067` confirmed it by reading it), and failover, write
forwarding and membership are a different system.

## Decision

- `RRF-FR-001` — `refresh(target, root, domain)`: connect with the
  replication token, `fetch_snapshot`, and install the files as
  `<root>/<secs>-<pid>-<seq>/`, crash-safely: every file written and
  synced under a staging directory, the staging directory synced and
  renamed into place in one step, `root` synced. A crash at any point
  leaves either no new directory or a complete one.
- `RRF-FR-002` — the installed directory is verified by a real reopen
  through the domain's own portable constructor; a directory that
  fails it is renamed under a `.failed-` prefix, never a name a
  refresh loop would pick, and the error names it.
- `RRF-FR-003` — a wrong token or a refused fetch writes nothing.
- `RRF-FR-004` — `snapshots(root)` lists the directories a refresh
  made, oldest first, by their name's three numbers and nothing else;
  `prune(root, keep)` removes all but the newest `keep`, and `keep ==
  0` removes nothing, so a loop can never delete what it just made.
- The CLI: `REPLICA_REFRESH_TOKEN=<token> replica_refresh <host:port>
  <root_dir> <memory|entity|relation> [--every <secs>] [--keep <n>]`.
  The token comes from the environment, never the command line, so it
  is not in the process list. One refresh by default, printing the
  directory to point `SERVER_DATA_DIR` at; `--every` repeats until
  interrupted; `--keep` prunes after each refresh. Needs the `client`
  feature; its test serves a real table and needs `server`.
- Not built, on evidence: TLS in the CLI. `ConnectOptions::tls` exists
  and the library's `Target.options` carries it; the CLI reads no
  certificate paths yet, so it runs on the server's host or a trusted
  network, said in its usage.

## Consequences

- Positive: a standby is refreshed by one command on a timer, with
  the same crash-safety the restore tool has, and a refresh that
  produced an unreadable directory cannot be mistaken for a good one.
- Negative / tradeoffs: still a full snapshot per refresh, bounded by
  `MAX_SNAPSHOT_BYTES`; the standby restart is still the operator's.
- Named, not hidden: no incremental shipping, no failover, no write
  forwarding, no membership — the growth document's list stands
  minus the script.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.97.0 / `FR-110`, in one PR with `ADR-0115`–`ADR-0117`, `ADR-0119`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 990 tests across 46 targets, 0 failed. Builder: Claude.
