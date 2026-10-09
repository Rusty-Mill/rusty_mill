# Sovereignty Audit: bbp-core against rusty_mill ADR-0002

Date: 2026-10-08. Rule applied: `Rusty-Mill/rusty_mill` ADR-0002, three dependency tiers. Target tier for bbp-core: **S, Sovereign**: no external normal or build dependencies; dev-only external dependencies allowed as independent test oracles.

## Normal dependencies

| Was | Now | Where in rusty_mill | Note |
| --- | --- | --- | --- |
| `serde` + `serde_json` | `rusty_serde` with `derive` | `crates/foundation/rusty_serde` | Zero crates.io deps, own derive macro, JSON format. No `HashSet` impl: `TaskState` and `Turn` lost their wire derives, which they never needed. `Sha256` serializes as 64 hex chars by a hand-written impl. |
| `sha2` | `rusty_rsa::sha256` | `crates/foundation/rusty_rsa` | Zero deps. The crate name is historical; SHA-256 is exported as a general hash. |

Result: `cargo tree -e normal` shows only `rusty_serde`, `rusty_serde_derive` (proc-macro workspace member, exempt), `rusty_serde_erased` and `rusty_rsa`. All 56 named tests, the model test and the proving run pass unchanged. A new `codec` test round-trips every logged event and the card through `rusty_serde` JSON.

## Dev dependencies

| Dep | Decision | Reason |
| --- | --- | --- |
| `proptest` | Keep, dev-only | ADR-0002 Tier S allows dev-only external dependencies as independent test oracles. rusty_mill has `cargo-fuzz` targets but no property-testing library, and a hand-rolled generator would lose shrinking, which found TR-31. |

## Stage 2 store

| Candidate | Verdict |
| --- | --- |
| SQLite (`rusty_sqlite`, bundled C) | No. Tier A by ADR-0002's own table, and BBP needs a log, not a relational store. |
| `rusty_multimodal_db_engine` | No for the log. It is an indexed record store carrying `uuid`, `thiserror`, `serde`, `bincode`, `memmap2`; its `journal` module is the nearest fit but ships the same dependencies. Using it would make bbp-core's store Tier T for no gain. |
| Own append-only log | **Yes.** One file per task of `rusty_serde` JSON lines, `fsync` on append, revision equals line count, plus a content-addressed blob directory written through `rusty_atomic_file` (foundation, zero deps). About 150 lines. Crash and reopen, conflict and blob-consistency tests as the stage-2 exit criteria. |

## Stage 3

- **MCP.** `rusty_mcp` exists at `libs/protocol` but is built on `rmcp` (Tier A by the ADR). bbp's MCP adapter is therefore an adapter crate, Tier A, kept out of bbp-core. A sovereign MCP server over stdio JSON-RPC is small enough to hand-roll if the mantra demands it; decide at stage 3.
- **Randomness.** Tokens and run secrets are deterministic hashes in stage 1. Stage 3 draws them from `rusty_rand` (foundation, OS CSPRNG, zero deps).
- **Process seam.** `orch-cli::CommandRunner` for the runner supervisor, per `review/orch-core-relationship.md`.

## Home

Path dependencies on rusty_mill crates require bbp-core to live inside that workspace. ADR-0002 names git-URL copies of workspace members as the divergence hazard it exists to prevent. The sovereign home is therefore `rusty_mill/crates/libs/protocol/rusty_bbp` with `layer = "libs"` metadata, under the workspace layer check, beside `rusty_routine`. The `rusty_bbp` repository keeps the spec, the reviews and the traces.

Done 2026-10-08 in Rusty-Mill/rusty_mill#542. The crate is `rusty_bbp` at `crates/libs/protocol/rusty_bbp`; this repository no longer carries code.
