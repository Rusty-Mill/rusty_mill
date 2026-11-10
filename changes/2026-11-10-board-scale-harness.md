---
category: Added
changelog: Mill Board scale harness: per-operation tick request, byte and payload ceilings on a production-shaped 1,425-card profile, plus a manual latency benchmark.
---
## 2026-11-10 - Mill Board scale harness and baseline

- **Added:** `crates/apps/rusty_mill_board/tests/scale.rs`. A CI test seeds the live board's 2026-11-10 size profile: 1,425 cards, sizes only, project names anonymised. It checks tick requests, tick response bytes and MCP payload bytes for `board_list`, `card_get`, `card_take`, `card_add` and `board_events` against recorded ceilings. Each bounded-reads slice (board cards #1426–#1431) lowers the ceilings it improves.
- **Added:** an ignored release benchmark that reports p50/p95 wall time at 1× and 4× scale, plus four concurrent readers and peak RSS. The baseline is in `docs/operations/scale-baseline.md`.
- **Changed:** the fake tick's loopback HTTP harness moved from `tests/large_tick.rs` to `tests/fake_tick/http.rs` and now counts traffic. `large_tick` behaves the same.
- **Known limitations:** no production code changes. The fixture tick is in memory, so its wall times are a lower bound on the deployed tick's.
