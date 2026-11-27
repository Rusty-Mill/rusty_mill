# #1551 Anchor the BBP claim-log head outside the host

Status: approved by Nano 2026-11-27 (D223); the board side is built (`rusty_mill_board` `anchor.rs`). Installing the guard line and setting the timer on BaileyAi wait for his separate host-change authorisation (D213, D214).
Follows #722 (PR #1485; D67-D70). Card: Mill Board #1551 (`bbp_factory`).

## Gap

Each BBP log line carries `prev`, the SHA-256 of the line before it, and `bbp verify` checks the
chain (D67, D70). Whoever can write the log (`orch-runner` on BaileyAi) can rewrite all of it and
recompute every hash. Catching that needs a copy of the head that `orch-runner` cannot change.

`bbp verify --expect H` already does the check half: it fails when `H` is not the hash of some
line, so a head saved earlier still passes after honest appends and fails after a truncation or
rewrite (D70). What is missing is a place to keep `H` and a moment to take it. The card's
`bbp verify --anchor` is not needed: `--expect` is that flag.

## Design

**Where.** A comment on the BaileyAi inbox board (`/var/lib/mill-board/board.db`, D175), not the
live board: on the card orch links there with `--card` (`execution.bbp_task`, the link the inbox
uses). The board writes it under the internal actor `bbp-anchor:mill-board`:

```
bbp-anchor v1 task=<T> lines=<N> head=<sha256 hex>
```

Why the board: it runs as its own user, `mill-board`, with its SQLite file in
`/var/lib/mill-board` 0700, outside the `orch-runner` group (D132). `orch-runner` cannot write it.
Comments are append-only (no edit or delete in `domain.rs`), so an anchor, once written, stays.
No new table, no schema migration, no new service.

**Who takes it.** The board pulls; the host never pushes. Through the existing guard, as for
`bbp inbox`, it runs:

```
bbp verify --dir /srv/lel/bbp --task <T> [--expect <last anchor head>]
```

and comments the printed `head` and `lines`. A pull means a rewritten log cannot slip in a
fresh anchor first: the board checks the old head before it records a new one.

**When.** On a timer, every `MILL_BOARD_BBP_ANCHOR_SECS` (off when unset or 0; BaileyAi sets 900
once Nano approves the design and separately authorises the host change, D214), for every task the inbox serves (cards with `execution.bbp_task`) and every
task that already has a trusted anchor, whatever its card's link or archive state now. Otherwise
`orch-runner`'s Build token could stop anchoring by unlinking or archiving the card. A task is
anchored only when its head changed since the last anchor, so an idle task adds no comments. The window an
attacker gets is history newer than the last anchor; everything older is pinned.

**On a mismatch.** `bbp verify` exits non-zero (broken chain, partial line, or the last anchor
missing). The board writes `bbp-anchor v1 task=<T> FAILED: <bbp's stderr line>`. It keeps running
verify each tick, still against the last good anchor, and comments again only when the failure
text changes or a head verifies again. A torn tail left by a crash also reads FAILED until the partial
line is gone. The inbox row is unchanged (a broken chain already fails every load,
D68).

**Manual check.** Anyone can read the last anchor with `card_get` and run
`bbp verify --dir D --task T --expect <head>` themselves.

**Trust rule.** An anchor counts only if its author is `bbp-anchor:mill-board` and its `task=`
is the task being checked. `:` is outside the characters `valid_actor` admits, so no credential,
`base/label` or registration can act under that name, and `orch-runner`'s board token
(`RUSTY_ORCH_BOARD_TOKEN`) cannot forge one. No name reservation, so no API change. A lookalike by
another actor is ignored.

## Changes

| Where | What |
|---|---|
| `rusty_mill_board/src/anchor.rs` (new) | Pure: parse anchor comments, pick the last trusted one, decide skip / anchor / fail from `bbp verify`'s result. Unit-tested. |
| `rusty_mill_board/src/bbp_cli.rs` | `verify(task, expect)` on the `BbpCli` adapter. |
| `rusty_mill_board/src/main.rs` | The timer thread and `MILL_BOARD_BBP_ANCHOR_SECS`. |
| `1092-deploy-board.sh`, `1092-deploy-layout.md` | The guard admits `verify` beside `inbox` and `human`. |
| `rusty_bbp`, `rusty_bbp_host` | None. No log format or BBP API change. |

## Limits

- Root on BaileyAi can rewrite both the log and the board file. The board is a different user, not
  a different machine. Covering root needs a copy off the machine, deferred by D215 until outside
  or untrusted agents join.
- A log made only of legacy unchained lines (#722, D69) has nothing to pin. The board anchors a
  task only when `verify` reports a chained head (`unchained < lines`). Once a chained line
  follows, its `prev` is the hash of the whole legacy prefix (`check` in `fs_store.rs`), so the
  prefix is covered like the rest.
- A task never linked to a card is not anchored. Orch links one when run with `--card`.
- Anchoring is a poll. BBP coordination is meant to be event-based; this poll is the board
  watching the log, not agents coordinating, and never touches BBP's control flow.

## Open questions

1. **Same machine or off it?** Decided by Nano, 2026-11-27 (D215): option c, "board only now,
   off-machine later". Anchors live on the BaileyAi inbox board, covering the harness running as
   `orch-runner`. An off-machine copy (remind-me or the live board) is added when outside or
   untrusted agents join, the trigger D67 set for signing. Rejected: a) board only with root out
   of scope for good; b) mirror off the machine now, which would put an outside-service write
   credential on BaileyAi. This settled Q1 only; the design was approved later (D223).

Settled as defaults by the let-em-loose-expert:
- The guard admits `verify` with D132's `--dir` rules, `--task` checked as for `inbox`, and
  `--expect` only as 64 lowercase hex (D213). Changing it on BaileyAi needs Nano's design approval and his separate host-change authorisation (D128, D131).
- The timer env var is off unless set; BaileyAi sets 900 under the same two approvals (D214).
