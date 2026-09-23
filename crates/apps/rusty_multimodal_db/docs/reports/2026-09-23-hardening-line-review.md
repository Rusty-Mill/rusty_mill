# Review of the hardening line, `ADR-0092` through `ADR-0109` (2026-09-23)

- Scope: every change merged on `claude/pr-276-multimodal-db-growth-4lkjx8`
  since the release-readiness review — PRs #292–#311, base commit
  `e2efa15`, head `016b501` (PR #311, `ADR-0108`/`ADR-0109`).
- Method: the diff was split into five areas and each given to a fresh
  reviewer with no memory of building it, asked for correctness
  findings with a concrete failure scenario, marked CONFIRMED (the code
  path read end to end) or PLAUSIBLE. Findings below are consolidated,
  de-duplicated, and re-ranked; two of the three Highs were found
  independently by two reviewers.
- Verdict: **the line is sound in its main mechanisms** (locking,
  refusal pool accounting, protocol append-only discipline, gating of
  the CI jobs) **and has three High findings that must be fixed before
  the crate is relied on for MVCC or for a multi-client deployment.**
  None of the three is on the consumer's current single-connection,
  non-MVCC path.

## High — fix now (*all three fixed by `ADR-0110`, the round after this report*)

1. **An in-place `UpdateField` is recorded in the MVCC index but never
   flushed to `.mvcc`, so a restart forgets it** (`memory.rs`
   in-place arm of `update_field`; the same in `entity.rs`,
   `relation.rs`). `apply_transaction`'s non-journaled arm calls
   `mvcc_flush_now(0)` after its record precisely because "a restart in
   between silently loses it"; `ADR-0108`'s arm claims to mirror it and
   does not. Scenario: MVCC-active table, `UpdateField access_count=41`
   acknowledged (with or without synced updates), restart through
   `open_with_mvcc`; the insert log carries no field updates, so
   `mvcc_get` at a fresh snapshot answers the pre-update value while
   `get` answers 41 — the inverse of the bug `ADR-0108` closed. Found
   independently by the durability and MVCC reviewers. CONFIRMED.
   Fix: flush after the record on the in-place arm, as the batch arm
   does, or record the update in a replayable log.
2. **A snapshot is registered after the index lock that read its
   `last_committed` is released, so a reclaim can drop the entry it
   needs** (`memory.rs` `mvcc_begin`, same in `entity.rs`,
   `relation.rs`, against `mvcc.rs` `with_index`'s trigger and
   `reclaim`). Scenario: A's `BeginWith` reads `S`; B's write appends
   `S+1` and crosses `reclaim_every` (default 10,000) → `gc(None)`
   keeps only `S+1` and sets `reclaimed_through=S+1`; A registers `S`;
   A's `Get` → `HistoryReclaimed` → a spurious `Conflict`. Pre-existing
   since `ADR-0096` (`Compact` only), moved onto every write path by
   `ADR-0105`. Found independently by the durability and MVCC
   reviewers. CONFIRMED. Fix: register the snapshot inside the same
   `with_index` call (lock order index → open_snapshots is already the
   trigger's).
3. **The Rust client's pre-hello reconnect turns a silent refusal into
   a version-1 session** (`client.rs` `connect_with`, `require_hello`
   false by default). `SERVER-002` §6.3 and `ADR-0104` say a close
   without the `Busy` frame is a failed connect; the client's `FR-026`
   fallback treats an EOF under `Hello` as a pre-hello server and
   re-dials speaking version 1 — if that dial is admitted, the whole
   session runs at version 1 against a version-30 server (`StrList`
   stripped, no `Join`, no `RowsClamped`) with no error surfaced.
   CONFIRMED. Fix: default `require_hello` to true now that every
   server speaks it, or make the fallback re-send `Hello` and fail if
   the second dial does not answer it.

## Medium — fix in the next round (*4–8 fixed by `ADR-0111`; 9–14 by `ADR-0112`*)

4. **`dogserver_query_rows_clamped_total` counts before
   authentication and before validation** (`serve.rs`, the clamp step
   right after `read_message`). An unauthenticated peer can drive the
   counter; a `Query` refused `Malformed` counts as clamped. Fix:
   count at the `mark_clamped` site, when the answer is `Rows`.
5. **The `RowsClamped` mark and `last_clamp` mean "a cap was imposed",
   not "rows were truncated"**, but the `protocol.rs` doc, the Rust
   and Python `last_clamp` docs, and `SERVER-002` §7 item 27 all say
   the stronger thing. A 5-row table under the default cap answers
   `RowsClamped{cap:10000}`. Either mark only when `rows.len() == cap`
   or fix the four docs. Two reviewers.
6. **`last_clamp` goes stale across the point-read and `ORDER BY`
   paths of `query`**, which return rows without touching it; the doc
   says every rows answer resets it.
7. **The metrics HTTP listener bypasses the exposure check**
   (`memory_server.rs`): `SERVER_METRICS_HTTP_ADDR=0.0.0.0:9100` with a
   loopback wire bind serves counters and table names to the network
   with no warning and no `SERVER_ALLOW_INSECURE` gate.
8. **The refusal thread's drain loop has no deadline**: each
   successful read restarts the 2 s timeout, so sixteen peers sending a
   byte every 1.9 s pin the whole refusal pool and every later refusal
   is a silent close. Bounded thread count, unbounded lifetime. Fix: an
   `Instant` deadline or a byte budget.
9. **`history_len` walks every chain under the index lock on every
   `Metrics` request and scrape** — `ADR-0109`'s "briefly" is untrue
   at scale; a running counter (the index already keeps
   `appended_since_gc`) makes it O(1).
10. **The automatic reclaim's counter includes replayed and reloaded
    entries and the trigger fires on any index access**, including
    reads and `mvcc_begin`'s baseline seeding; harmless to data (every
    `gc(None)` keeps the newest entry) but `auto_reclaims` lies and
    `ADR-0105`'s "runs inside the committing session" is false for
    every path but a session commit.
11. **Two creation sites and one rename lack `sync_parent_dir`**: the
    journal's and the insert log's creation, and `handle_backup`'s
    rename of the backup directory — `ADR-0092` promises every rename
    install. PLAUSIBLE on the creations, CONFIRMED on the backup.
12. **`journaled_update` answers `Ok(false)` for any apply error after
    the entry is fsynced**; a `Delete` racing between the pre-journal
    `validate_batch` (outside the exclusive section) and `apply_batch`
    leaves a journal entry for a deleted record that replay will retry.
    PLAUSIBLE.
13. **`ErrorCode::TooLarge`'s doc and `with_max_query_rows`'s doc still
    say a `Query` with no `limit` is refused**; it has been clamped
    since `ADR-0102`.
14. **`Cargo.toml`'s `rust-version` comment still names the deleted
    standalone workflow**; and the MSRV job's display name embeds the
    pin literal, so a pin bump renames the check any name-based branch
    protection would require. PLAUSIBLE on the protection.

## Low — housekeeping (*fixed by `ADR-0112` except the items it names as recorded, not changed*)

15. `SERVER_JOURNAL_UPDATES` is presence-gated while
    `SERVER_SYNC_UPDATES` is a `0`/`1` switch two lines away;
    `SERVER_JOURNAL_UPDATES=0` turns it on. Make it a switch.
16. `bounded_env` panics on an exported-but-empty variable and gives
    the wrong message on integer overflow.
17. A page with a bad `order_by` and a limit over the cap now answers
    `TooLarge` where it answered `UnknownField`; and a library caller's
    `with_max_query_rows(0)` clamps every limitless `Query` to zero
    rows. Document or refuse zero.
18. Prometheus label values are interpolated unescaped in
    `render_metrics`; shipped table names are fixed, so no impact today.
19. A poisoned index mutex takes down every `Metrics` and scrape;
    `thread::spawn` (not `Builder::spawn`) panics the accept loop at
    the process thread limit. Pre-existing patterns.
20. The Python client leaks the socket on a `Busy` (or any) failure in
    `connect`, pinning a server refusal slot for up to 2 s while the
    exception lives; no Python test covers `Busy` or `last_clamp`; a
    stale "declares protocol 27" comment.
21. Test names and messages that say the opposite of what they assert:
    `a_connection_past_the_cap_is_closed_at_accept_and_counted` (now
    asserts a `Busy` frame); "stays open past its one frame" on an
    `is_err()` that proves EOF; "nothing reclaimed with a snapshot
    open" on a `<`; "the fourth append crossed the threshold" when the
    third did; `allow_insecure_turns_the_refusal_into_a_warning…`
    observes only a TCP connect; the data-dir-lock test asserts
    connect failure after the child exited. Entity/Relation arms of
    `ADR-0108` are untested; the "racing session sees `Conflict`"
    benefit is not asserted.
22. `SERVER-002` §6.3 still says "exactly two cases" above a paragraph
    adding a third. `ADR-0097` says the `msync` runs "under the write
    lock" of the update; it takes a second lock. `ADR-0092`'s
    directory-durability claim is unqualified off Unix. `ADR-0072` says
    GC is "used only by `Compact`".
23. The root feature-set job lints four feature sets; `ADR-0098` lists
    five (`server,research` is covered only by `--all-features`).
    Other Windows tests (`platform-windows` parity, rush job control)
    still shell out through an untrimmed PATH; the `set_var` trim in
    sessionmgr's test harness is a benign race under local `cargo
    test`. A "TEMP … not for merge" probe commit is in `main`'s
    history (tree clean).

## What the reviewers found sound

`DataDirLock` (create-without-truncate, `try_lock`, held for all of
`main`); every rename install that got `sync_parent_dir` orders file
sync → rename → directory sync; the lock order store → index →
open_snapshots is acyclic and the reclaim trigger cannot recurse or
double-fire; `try_admit` is race-free from the single accept thread
and the connection and refusal guards are symmetric; `refuse_busy`
does no I/O on the accept thread and the timeouts precede the
handshake; `downgrade_for_version`'s first arm re-enters so a
connection below 11 loses `StrList`; both golden vectors decode by
hand; the Python encoder matches the Rust field order; a protocol-29
connection never sees index 24; `bind_is_loopback` fails safe on
`0.0.0.0`, `[::]`, mapped IPv6 and hostnames; the root CI jobs are
gated by the affected-crate plan with reverse dependencies and run
on every push to `main`; no CI step passes vacuously; the MSRV pin
matches `Cargo.toml`; the Windows `PING_30S` commands resolve
correctly under a truncated PATH.

## Recommended order

1. Highs 1–3 in one round (MVCC flush on the in-place arm; register
   the snapshot under the index lock; the client's fallback).
2. Mediums 4–8 in a second round (counter site, the mark's semantics,
   `last_clamp`, the metrics listener's exposure, the drain deadline).
3. The rest as housekeeping alongside whatever touches those files.
