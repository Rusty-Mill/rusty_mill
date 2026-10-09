# Decision Record: bbp-core and orch-core

Date: 2026-10-08. Trigger: stage 1 of bbp-core passing, the point the implementation-plan review (P12) set for this decision. Source read: `Rusty-Mill/rusty_mill` at `crates/apps/rusty_orch` (orch-core 1,565 lines, orch-dispatch 718, orch-cli 993, orch-store 959, adapters, 12 ADRs).

## Correction to round one

My round-one review (F1) said BBP "re-derives about 70% of orch-core". Having read the code, that was wrong. The shared surface is the Board's shape, about 200 lines: append-only entries, validated refs, and ADR-0005's decision policy. The remaining 1,300 lines of orch-core are a goal contract and a task-card `Plan`, which BBP does not have and does not need. The two cores sit at different layers.

## What each one is

| | orch-core + dispatch | bbp-core |
| --- | --- | --- |
| Unit | A goal decomposed into a DAG of task cards | One software task from spec to merge |
| Agents | Routed by role to a CLI adapter; the dispatcher calls one agent at a time, renders the board into the prompt, and appends the reply | Agents pull through MCP under an execution token; the store rejects what rules forbid |
| Record | `Board` of 7 entry kinds with supersession; snapshot persistence | Event log with blob, artifact, message and event records; `TaskState` fold |
| Control | `Plan` lifecycle per card, `Ledger` call counts, human answers unblock questions | Ten-state transition table, two human gates bound to spec and run, runner evidence, budgets per field |
| Visibility | All agents see the live board | Per-role capability matrix; Reviewer isolated |
| Deps | none | serde, serde_json, sha2 |

orch-dispatch is the relay loop BBP v0 set out to replace: the dispatcher reads the board on the agent's behalf and pastes it into a prompt. That is the design, not a defect, and it is what makes orch-core unsuitable as a starter for a pull-based, token-fenced store.

## Decision

**Compose, do not rebase.** bbp-core stays as built. The two meet at a boundary, not inside one crate:

1. **rusty_orch above BBP.** A `Plan` card that produces code becomes one BBP task. rusty_orch opens it, assigns roles, drives the human channel, and reads the card to know when the task closed. The `Goal` contract and `Plan` stay in orch-core unchanged.
2. **orch-core's Board retires in favour of the BBP store**, once rusty_orch's adapters can post to BBP instead of returning `Output` to the dispatcher. Until then the Board stays for research and design cards that produce no code. This is the dedupe answer: one blackboard in the ecosystem, reached when the MCP adapter exists (stage 3), not before.
3. **Reuse now, by dependency, not by copy:**
   - `orch-cli::CommandRunner` and `StdCommand` (hard deadline, group kill, env scrub, working directory) for the runner supervisor and for launching agent processes in stage 3.
   - `rusty_multimodal_db_engine` as the stage-2 durable store, registry-free, instead of SQLite. ADR-0010 shows the pattern; BBP needs an append-only event log with a revision check rather than a snapshot per goal, so the adapter is new but the engine is shared.
   - ADR-0005 verbatim as the basis of BBP Appendix B when cross-vendor decisions return.
4. **Home.** bbp-core moves into `rusty_mill` as a libs-layer protocol crate (`crates/libs/protocol/bbp_core`, beside `rusty_routine`) when stage 2 starts, so the orch family can depend on it under the workspace layer check. rusty_bbp keeps the spec, the reviews and the traces.

## What this costs

Two cores exist for one more stage. The Board and the BBP message store overlap on about 200 lines until stage 3 retires the Board. Accepted: the alternative, rewriting the Board to carry tokens, typed verdicts, per-role visibility and runs, is a rewrite of orch-core wearing its name.

## Open

- Whether rusty_orch's three CLI adapters become BBP agents by running under a BBP MCP server, or whether the orch dispatcher itself becomes a BBP principal that proxies them during the transition. Decide at stage 3.
