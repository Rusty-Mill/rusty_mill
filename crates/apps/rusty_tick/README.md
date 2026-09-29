# rusty_tick

A self-hosted, TickTick-style task manager built on
[`rusty_multimodal_db_engine`](../../libs/storage/rusty_multimodal_db_engine).
A clean-room design: it is written from the product's public behaviour and
documented API, not from its code.

Status: **storage spike only**. `TaskStore` (tasks, per-list ordering, due-date
ranges, full text, tags) and probes for the engine gaps in
[#382](https://github.com/Rusty-Mill/rusty_mill/issues/382). Results are in
[SPIKE-FINDINGS.md](SPIKE-FINDINGS.md). No HTTP API or UI yet.

```
cargo test -p rusty_tick
cargo test -p rusty_tick --release --test spike -- --ignored --nocapture
```
