# Independent review of the growth line, `ADR-0110` – `ADR-0119`

- Date: 2026-09-23, on the owner's "2" (review before the next area).
- Scope: everything merged since the last review
  (`2026-09-23-hardening-line-review.md`, which stopped at `ADR-0109`):
  the review's fix rounds (`ADR-0110`–`ADR-0112`), the journal-plus-MVCC
  composition (`ADR-0113`, `ADR-0114`), and the five growth items
  (`ADR-0115`–`ADR-0119`), PRs #312–#318.
- Method: five fresh reviewer agents, one area each, read-only, each
  told to verify by reading the code path and to mark every finding
  CONFIRMED or PLAUSIBLE. Their reports were consolidated here without
  softening; the numbering is by severity across all five.
- Verdict: the mechanisms are sound and the tests of these rounds
  prove what their ADRs claim, with three exceptions that matter — one
  High in the MVCC reopen, and two Highs in the operator tooling that
  make a shipped runbook and a shipped hint wrong. Nothing found
  changes the readiness verdict of 2026-09-21; the High below is on a
  path the binary takes when MVCC is first enabled on an existing
  table, so it goes first.

## High — fix now (*all three fixed by `ADR-0120`, the round after this report*)

1. **`attach_mvcc` folds the pending insert log into an inactive index,
   so the later baseline seed lands below it and a fresh snapshot reads
   a stale value.** `src/server/memory.rs` `attach_mvcc` /
   `fold_pending_log_entries` (no `is_active` gate; `entity.rs`,
   `relation.rs` the same); `MvccState::open`'s own doc says the adapter
   must fold nothing until activation. Scenario: a table never
   MVCC-active; ordinary `Insert X`, then an in-place `UpdateField`
   `X.count = 7`; restart with `SERVER_MVCC_ISOLATION` → the pending
   `X count=0` is folded at txn 1; the first `Begin` seeds the baseline
   `count=7` at txn 0, below it; a fresh snapshot reads 0 while
   `GetById` reads 7. The replay fold already gates on `is_active`; the
   pending fold must too. CONFIRMED.
2. **`scripts/power_loss_trial.sh` cannot test a mark: `replay-log
   --list` does not exist.** xfstests' `replay-log` has no `--list` and
   no mode that prints mark names; with `2>/dev/null` the usage error is
   hidden, the `for` word list is empty, and the script exits 0 having
   replayed nothing. `--log`/`--replay`/`--end-mark` are real. The
   script already knows its mark names per mode; iterate those.
   Also (#6 below) the replay writes over the device's *final* state,
   so post-mark blocks survive unless the device is zeroed first.
   CONFIRMED.
3. **The old-layout refusal hint tells the operator to point
   `SERVER_DATA_DIR` at a slot file, and following it silently drops
   the entity and relation tables.** `src/bin/memory_server.rs`
   `open_error`: `migrate_memory_v1_to_v2 <old_path> <new_path>` takes
   the `memories.mmap` path, `SERVER_DATA_DIR` is a directory that gains
   fresh empty `entities.mmap`/`relations.mmap`. The test pins only the
   substrings. CONFIRMED.

## Medium — fix in the next round (*all twelve fixed by `ADR-0121`; item 11's test found the real fix was `Compact`'s post-clear flush*)

4. **`dogserver_journal_waiting_writers` reads 0 during a stalled
   `fsync`.** `src/server/journal.rs` `stats` counts `turn_waiters`,
   which a writer enters only after its sequence is durable; followers
   wait on the `durable` condvar and are never counted. `ADR-0115`'s
   "an alert on `waiting_writers` catches a stalled `fsync`" is not
   delivered. Count condvar waiters too. CONFIRMED.
5. **Under TLS the refusal drain deadline does not bound the handshake;
   a trickling peer pins a refusal slot for hours.** `serve.rs`
   `refuse_busy`: the `Instant` deadline is set after the frame is
   written; `complete_handshake` restarts the 2 s read timeout per
   record read. Sixteen such peers exhaust `MAX_BUSY_REFUSALS`.
   CONFIRMED.
6. **`power_loss_trial.sh` replays onto a device that still holds
   post-mark blocks.** Zero `$LOOP` or replay onto a fresh image before
   each mark. PLAUSIBLE.
7. **The metrics HTTP listener uses `thread::spawn` with no cap and no
   read timeout; at the thread limit it panics and `/metrics` is gone
   until restart.** `src/server/metrics_http.rs`. `RVL-FR-008` was not
   applied there. CONFIRMED.
8. **Both clients send `Null` to a server below 31, which closes the
   connection with no reply** (rule 4). Python `_to_scan_value` maps
   `None` unconditionally; the Rust client has no value-variant gate.
   Refuse locally when the negotiated version is below 31. CONFIRMED.
9. **`downgrade_for_version` strips `Null` only from `Record`/`Rows`;
   `JoinedRows`, `Groups`, `ScanValues` are uncovered and have no
   assertion.** Latent until a nullable column ships; extend the strip
   to `JoinedRows` now or name the gap in `ADR-0117`. PLAUSIBLE.
10. **The open-time fold is flushed after the sources it depends on are
    destroyed.** `with_journal` truncates the journal and the reopen
    clears the insert log before `attach_mvcc` flushes `.mvcc`; a crash
    or flush failure in that window loses the batch from the index
    permanently. Small window, unrecoverable. CONFIRMED.
11. **No test proves the `ReplayedBatch::Write` fold arm**; the unit and
    binary tests pass with it removed, because every journaled write
    also sits in the pending insert log. The arm is needed after a
    `Compact`; a test that compacts first would cover it. CONFIRMED.
12. **`snapshots()`/`prune()` match any `N-N-N` directory name**, e.g.
    an ISO date `2026-09-23`, and `prune` deletes it first. CONFIRMED.
13. **A failed `.failed-` rename leaves a bad snapshot under a good
    name**, and the error names a path that does not exist. PLAUSIBLE.
14. **Snapshot file names from the server are joined unvalidated**; an
    absolute or `..` name escapes the staging directory. The honest
    server sends flat names; the transport is plaintext. Reject any
    name that is not a single normal component. CONFIRMED (path),
    PLAUSIBLE (threat).
15. **Doc drift.** `src/lib.rs` says protocol 18; `PROJECT-STATUS.md`'s
    "Current milestone" line is 26 rounds stale; `TRACEABILITY.md`'s
    PLP row contradicts itself on whether the trial ran;
    `memory_server`'s module docs omit `SERVER_METRICS_HTTP_ADDR`.
    CONFIRMED.

## Low — housekeeping (*fixed or recorded in place by `ADR-0122`; three tests not added, named there*)

16. Persisted `.mvcc` chains folded at open count toward the automatic
    reclaim (`MvccState::open` never resets the append count).
17. `with_index`/`with_index_quiet` `lock().unwrap()` panic the scrape on
    a poisoned index; `CommitGroup::stats` reads zeros instead.
18. `SERVER_METRICS_HTTP_ADDR=""` is a startup refusal, not "unset".
19. `escape_label_value`'s doc comment is split across two items;
    `ADR-0115` says `CommitGroup::len_bytes` is no longer test-only (it
    is); duplicate table names in `serve_tables` yield duplicate series.
20. Both trial scripts hang if the writer dies before its awaited line;
    `crash_writer reopen-check` without a path panics rather than
    exiting 1; staging and `.failed-` directories are never cleaned; a
    backwards clock step lets `prune(keep=1)` delete the newest.
21. `RESULTS.md`'s "does not consult the page cache" overstates the
    crash-prefix trial: jbd2's 5 s commit or background writeback can
    land before the copy, and `cp` is not atomic against a commit.
22. `ADR-0117`'s "Malformed wherever a value is read" is over-stated: a
    read-only or unknown field answers its own code first, as for any
    wrong-kind value; §4's `Hello` example and rule 3's prose still say
    30/StrList; a stale doc comment in `protocol.rs` tests.
23. `ADR-0113`/`0114`'s premise "the journal's entries are younger than
    any pending insert" is false when a non-journaled write follows a
    journaled one; the result is a consistent but lost update,
    pre-existing from `ADR-0025`'s replay semantics.
24. `memory_server`'s `ReopenWithMvcc` arms bypass `open_error`'s
    remedy; `mvcc_begin` discards the baseline flush result.
25. Back-references missing: `SERVER-REPLICATION-DESIGN.md` still says
    no script ships; `ADR-0056` lacks an `ADR-0117` note; the crash
    harness's caveat does not point at the power-loss design;
    `FUTURE-GROWTH` line 117 lists the journal metrics as absent before
    correcting itself; `README.md` attributes the row cap to `ADR-0093`.
26. Tests: `waiting_writers` is only ever asserted 0; `.failed-` and
    `--every` are untested; "the old directory is untouched" checks
    existence only; the allow-insecure metrics-bind test does not
    assert the warning names the variable; `free_port()` is a port race
    under parallel runs.

## What the reviewers found sound

The clamp counter and mark; the owned in-flight and refusal guards;
plaintext refusal FIN and drain; lock ordering between render, commit
and index; Prometheus exposition format and label escaping; the
exposure check; env parsing; O(1) `history_len`; `open_snapshot`/
`reclaim` under the index lock; txn-id ordering of the open-time folds;
`RVL-FR-004` replay; `Null`'s index, vectors, Python codec, every
server-side refusal path and the session overlay; `require_hello`;
`replica_refresh`'s token handling, sync/rename ordering and prune
bounds; both scripts' teardown; every cited path exists; the
`SERVER-001`/`SERVER-002`/registry version chain is consistent.

## Recommended order

1. High 1 (one-line gate plus a test that fails without it), High 3
   (the hint's path and a test that pins the directory semantics),
   High 2 with Medium 6 (the runbook's marks and a zeroed device).
2. Mediums 4, 5, 7 (the three server ones), then 8, 9 (the wire pair),
   then 10, 11 (the reopen flush order and the missing Write-arm test),
   then 12–14 (the refresh CLI), then 15 (doc drift).
3. The Lows, in one round, with the report annotated.
