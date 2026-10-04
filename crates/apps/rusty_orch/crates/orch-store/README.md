# orch-store

Persistence for the orchestrator ([ADR-0010](../../docs/adr/0010-persistence-on-multimodal-db.md)): the plan, board, and ledger of one goal saved as a single snapshot record in the embedded `rusty_multimodal_db` engine (`crates/libs/storage/rusty_multimodal_db_engine`), so a run that stops blocked in one process resumes in the next.

```rust
let mut store = Store::open(Path::new(".orch"))?;          // locks the directory
let state = store.load(goal)?;                              // Option<GoalState>
store.save(&GoalState { fingerprint, plan, board, ledger })?; // durable on return
```

- **One record per goal.** A save is one `insert` or `replace`; the engine makes it durable before returning. Nothing has to land together, so there is no journal.
- **Rebuilt, not trusted.** `load` replays the snapshot through `Plan::add` and the lifecycle transitions, `Board::append`, and `Ledger::from_counts`, so every `orch-core` invariant is checked again. A snapshot that fails is `StoreError::Corrupt`.
- **Fingerprint.** `fingerprint(goal_json)` is stored with the snapshot; the caller refuses to resume when the goal file no longer matches.
- **One process per directory** (`StoreError::Locked`).

The engine's record bound needs `serde` derives on the snapshot rows, which makes this the one crate in the family with registry dependencies, and its `rust-version` is the engine's 1.89. The initial format is tagged `rusty_orch::GoalRecord::v1`. Positional bincode makes the complete row layout the format; adding fields is not compatible. A future incompatible layout must either have an explicit supported reader that migrates v1 or use a new tag and reject v1 before decoding.

Plan, board, and ledger share one record payload and therefore cannot come from different snapshots. The revision is narrower: it is the persisted scannable checkpoint generation, not a count of successfully returned saves. If replacement fails after its durable log append but before its separate scannable slot update, portable reopen can recover the new aggregate payload with the preceding revision. An initial insert interrupted after its durable log append can reopen with a synthesized generation 1 even though `save` returned an error. Only a successful `save` return guarantees the new payload and revision are durable.

```sh
cargo test -p orch-store
```
