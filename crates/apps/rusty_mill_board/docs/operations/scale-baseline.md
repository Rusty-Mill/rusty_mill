# Scale baseline: storage traffic and latency per operation

This is the before-and-after record for the bounded-reads work: board card #1426 and its children, S0–S3 and S6. Each slice that bounds an operation adds a row to the per-slice table, and lowers that operation's ceiling in `tests/scale.rs`.

## Dataset

`tests/fixtures/board-profile-2026-11-10.json` is the size profile of the live board on 2026-11-10. It has one entry per card, oldest first: project (anonymised `p01`… by card count), column, and the lengths of the title, body, prompt and each comment. It holds no card text.

| Measure | Value |
|---|---:|
| Cards | 1,425 (754 open, 671 done) |
| Bodies | 746 KB |
| Prompts | 1.24 MB |
| Comments | 921 KB |
| Projects | 27; largest 145 cards; `p03` = 112 |

The test seeds the fake tick (`tests/fake_tick`) with that profile and serves it over loopback HTTP (`tests/fake_tick/http.rs`). The board reads it through `TickStore` with the default production client and the same paged collections it uses in production (PR #1568).

In the fixture, Done cards carry `board:done` on an open task and have no completion time. The default `board_list` therefore shelves them, as production does for Done cards older than 14 days.

## How to run

```sh
# CI guard: tick requests, tick bytes and MCP payload per operation vs CEILINGS
cargo test -p rusty_mill_board --test scale

# Latency benchmark (release, about 1 min): p50/p95 over 20 warm runs at 1x and 4x
cargo test --release -p rusty_mill_board --test scale -- --ignored --nocapture
```

- **Payload** is the JSON size of the MCP tool result as the API returns it.
- **Tick bytes** are response-body bytes the fixture sent.
- **Wall time** is only reported, never asserted.

## Baseline: main f06584c0 (2026-11-10)

Release build, 1×:

| Operation | p50 | p95 | Tick requests | Tick bytes | Payload |
|---|---:|---:|---:|---:|---:|
| `board_list` | 82.9 ms | 96.6 ms | 7 | 3,692,846 | 1,960,215 |
| `board_list project=p03` | 75.1 ms | 93.4 ms | 7 | 3,692,846 | 160,758 |
| `card_get` by id | 4.3 ms | 7.5 ms | 5 | 4,645 | 2,021 |
| `card_get #700` | 9.6 ms | 19.0 ms | 11 | 9,774 | 2,021 |
| `board_events` (steady poll) | 29.6 ms | 36.2 ms | 2 | 1,239,017 | 27 |
| `card_take` (CI test, debug) | — | — | 33 | 3,724,888 | 3,016 |
| `card_add` (CI test, debug) | — | — | 11 | 3,694,995 | 465 |

Four concurrent `board_list`: 307.5 ms wall.

At 4× (5,700 cards):

| Operation | p50 | p95 | Tick requests | Tick bytes | Payload |
|---|---:|---:|---:|---:|---:|
| `board_list` | 795.6 ms | 822.2 ms | 22 | 14,787,766 | 7,846,284 |
| `board_list project=p03` | 696.4 ms | 734.3 ms | 22 | 14,787,766 | 643,184 |
| `card_get` by id | 18.2 ms | 23.1 ms | 5 | 4,645 | 2,021 |
| `card_get #700` | 38.4 ms | 50.4 ms | 11 | 9,774 | 2,021 |
| `board_events` | 297.1 ms | 364.6 ms | 7 | 4,966,653 | 27 |

Four concurrent `board_list`: 2.6 s wall. Peak RSS for the fixture, server and board in one process was 303 MB.

What this shows:

- **Filters do not reduce reads.** A project-filtered list reads exactly what an unfiltered one reads; only the payload shrinks.
- **Writes read the whole board.** `card_take` and `card_add` each read about the whole board (3.7 MB).
- **Idle polls are expensive.** A `board_events` poll reads every agent task (1.24 MB) even when nothing changed.
- **Growth is faster than linear.** Four times the cards costs about ten times the wall time for full reads.
- **`card_get #N` reads the card twice** (11 requests against 5 by id). The number resolves to a card, then the operation reads it again by id. This is cheap to remove; S1 or S3 can take it.

These are lower bounds for production: the fixture tick is in memory, and there is no tunnel or remote MCP client. Production timings come from S0 (card #1428).

## Per slice

| Slice | PR | Operation | Requests | Tick bytes | Payload | p50 (1×) |
|---|---|---|---:|---:|---:|---:|
| S6 baseline | #1607 | see above | | | | |
| S1 summaries (#1429) | this PR | `board_list view=summary` | 7 → 3 | 3,692,846 → 1,240,682 | 1,960,215 → 207,451 | 71.7 → 32.9 ms |
| S1 summaries (#1429) | this PR | `board_list view=summary project=p03` | 7 → 3 | 3,692,846 → 1,240,682 | 160,758 → 12,288 | 58.7 → 26.7 ms |
| S1 paging (#1429) | this PR | `board_list limit=100` (first page, full view) | 7 | 3,692,846 | 1,960,215 → 219,641 | 71.7 → 65.3 ms |

S1 notes: a summary reads tasks only, 33.6% of a full read's tick bytes, just over the card's one-third target; the rest is task notes, which the summary drops but the task listing still carries. A full-view page bounds the payload, not the storage read. p50 compares full and summary in the same release run on this branch (full `board_list` 71.7 ms here against 82.9 ms in the S6 run; same code, different machine load). At 4× a summary is 364.9 ms against 759.6 ms full, and a `p03` summary 315.1 ms against 636.1 ms: halved, but still linear in the board, because the task listing is read whole. S4 (tick-side filters) is what bounds that.
