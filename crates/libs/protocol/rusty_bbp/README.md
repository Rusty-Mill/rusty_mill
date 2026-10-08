# rusty_bbp

The Blackboard Protocol (BBP) core: a shared store through which AI agents from different vendors collaborate on one software task, with per-role visibility, evidence rules, budgets and human gates enforced by the store rather than by prompts.

Tier S under [ADR-0002](../../../../docs/adr/0002-dependency-sovereignty-policy.md): `rusty_serde` for typed payloads and the event log, `rusty_rsa` for SHA-256, `rusty_atomic_file` for crash-atomic blob files. The core (`engine`, `state`, `record`, `command`, `event`) does no I/O and reads no clock: `engine::handle(state, command, now)` returns the events to append and the response to return, and `TaskState` is the fold over events. `store` holds the storage port, the in-memory adapter and the `Driver`; `fs_store` is the durable adapter.

The specification, four rounds of cross-model design review, and the trace catalog this crate's tests reproduce live in [baileyrd/rusty_bbp](https://github.com/baileyrd/rusty_bbp). `TRACES.md` here is the catalog; `tests/traces.rs` has one test per row.

```sh
cargo test -p rusty_bbp
PROPTEST_CASES=3000 cargo test -p rusty_bbp --test model
cargo clippy -p rusty_bbp --all-targets -- -D warnings
```

Stages 1 and 2 of the implementation plan are done: the pure core with the in-memory store, and the durable store. Stage 3 adds an MCP adapter and a sandboxed runner supervisor.

## Durable store

`FsStore::open(dir)` keeps one append-only log per task at `tasks/<task>.log` and blobs at `blobs/<sha256 hex>`. Each committed batch of events is one JSON line, written and `fsync`ed as a unit, so a crash leaves at most one partial line and `open` truncates it: a transaction either fully lands or leaves no trace. The revision is the number of events in the complete lines; `append` re-reads the file and refuses a stale expected revision, which is how two handles on one directory are kept consistent, and the `Driver` reloads and recomputes on that conflict. Blobs are written through `rusty_atomic_file` and verified against their digest on every read. One writer per directory at a time is the host's responsibility; there is no lock file yet. Relationship to `rusty_orch`: that family plans goals into task cards and will open one BBP task per code-producing card; its Board retires in favour of this store once adapters post through BBP.
