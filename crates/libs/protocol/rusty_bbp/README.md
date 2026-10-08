# rusty_bbp

The Blackboard Protocol (BBP) core: a shared store through which AI agents from different vendors collaborate on one software task, with per-role visibility, evidence rules, budgets and human gates enforced by the store rather than by prompts.

Pure domain crate, Tier S under [ADR-0002](../../../../docs/adr/0002-dependency-sovereignty-policy.md): `rusty_serde` for typed payloads and the event log, `rusty_rsa` for SHA-256. No I/O, no clock, no randomness. `engine::handle(state, command, now)` returns the events to append and the response to return; `TaskState` is the fold over events; `store` holds the storage port and an in-memory adapter.

The specification, four rounds of cross-model design review, and the trace catalog this crate's tests reproduce live in [baileyrd/rusty_bbp](https://github.com/baileyrd/rusty_bbp). `TRACES.md` here is the catalog; `tests/traces.rs` has one test per row.

```sh
cargo test -p rusty_bbp
PROPTEST_CASES=3000 cargo test -p rusty_bbp --test model
cargo clippy -p rusty_bbp --all-targets -- -D warnings
```

Stage 1 of the implementation plan: in-memory store, scripted agents, stub runner, all through the normal API. Stage 2 adds a durable append-only log; stage 3 an MCP adapter and a sandboxed runner supervisor. Relationship to `rusty_orch`: that family plans goals into task cards and will open one BBP task per code-producing card; its Board retires in favour of this store once adapters post through BBP.
