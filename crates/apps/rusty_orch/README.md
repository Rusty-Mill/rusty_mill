# rusty_orch

A small orchestrator that lets Claude, ChatGPT (Codex), and local models (Ollama/Hermes) collaborate on development research and tasks without a human relaying context between them. Agents never message each other: they read and write a shared, append-only blackboard and a git repo, and a deterministic dispatcher hands each one a task card.

## Status
Experimental. Domain core (`orch-core`), an in-memory dispatcher (`orch-dispatch`), a shared CLI-adapter core (`orch-cli`), three real adapters (`orch-ollama` for `Agent::Local`, `orch-codex` for `Agent::Codex`, `orch-claude` for `Agent::Claude`, both read-only), a snapshot store (`orch-store`) over the embedded `rusty_multimodal_db` engine, and the `rusty_orch` command that runs a JSON goal file through them and resumes a blocked run from `--state <dir>`. Owner: @baileyrd.

## Getting started
```powershell
# from the rusty_mill workspace root
cargo test -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex -p orch-claude -p orch-store -p rusty_orch --all-features
ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-ollama --example research -- "How does Plan::start prevent self-review?"
ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-codex --example research_review -- "How does Plan::start prevent self-review?"
```

## Layout
| Path | What |
| ---- | ---- |
| `crates/orch-core` | Pure domain: goal contracts, task cards + `Plan`, blackboard. No I/O, no dependencies. |
| `crates/orch-dispatch` | Application layer: `AgentRunner` port, validated role → agent routing, sequential loop over `Plan` metered by a caller-owned `Ledger`. Depends only on `orch-core`. `FakeAgent` behind the `fake` feature. |
| `crates/orch-cli` | Shared CLI-adapter core: `CommandRunner`/`StdCommand` seam (hard deadline, group kill, env scrub, working directory), strict JSON `parse`, prompt `render` with a per-adapter footer, the reply `output_schema`, `fake` test doubles. The kinds a role may write depend on the goal's stop rule: best effort withdraws `question` ([ADR-0011](./docs/adr/0011-best-effort-stop-rule.md)). |
| `crates/orch-ollama` | `Agent::Local` over `ollama run --format json`: thin adapter on `orch-cli`. `research` example runs one card end to end. |
| `crates/rusty_orch` | The command: `rusty_orch run <goal.json> [--interactive] [--json]`. Parses the goal file, wires both adapters, enforces the wall clock, prints the board. ADR-0009. |
| `crates/orch-codex` | `Agent::Codex` over `codex exec --sandbox read-only`, schema-constrained reply via the last-message file, `OPENAI_API_KEY` scrubbed. `research_review` example: Codex researches, a local model reviews. |
| `crates/orch-claude` | `Agent::Claude` over `claude -p --tools Read,Grep,Glob --restricted`, schema-constrained reply read from the JSON result envelope, a `claude auth status` probe before each run, `ANTHROPIC_API_KEY` and provider overrides scrubbed ([ADR-0012](./docs/adr/0012-claude-adapter.md)). |
| `crates/orch-store` | Persistence: one snapshot record per goal (plan, board, ledger, goal-file fingerprint) in `rusty_multimodal_db_engine`, rebuilt on load through the domain's constructors. The family's one crate with registry dependencies ([ADR-0010](./docs/adr/0010-persistence-on-multimodal-db.md)). |

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) and [docs/adr/](./docs/adr/).

## Development
```powershell
cargo test -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex -p orch-claude -p orch-store -p rusty_orch --all-features
cargo fmt -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex -p orch-claude -p orch-store -p rusty_orch -- --check
cargo clippy -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex -p orch-claude -p orch-store -p rusty_orch --all-targets --all-features -- -D warnings
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md). Agents working in this repo: read [AGENTS.md](./AGENTS.md) first.

## Security
See [SECURITY.md](./SECURITY.md).

## License
Internal — not for external distribution.

## BBP implementation tasks

`rusty_orch run goal.json --bbp bbp.json --state /srv/orch/state` hosts Implement
cards on the existing BBP host. Its roster and provider pool are explicit and
frozen per task. Research and design retain their current adapters. See
[configuration, recovery and validation](crates/orch-bbp/README.md).

Add `--card <id> --board http://127.0.0.1:8787` to host the goal for a Millboard
card. Each run records its assignment generation with the card before any host
command, and stops if the card already carries a later one. The credential and
identity come from `RUSTY_ORCH_BOARD_TOKEN`, `RUSTY_ORCH_BOARD_ACTOR` and
`RUSTY_ORCH_BOARD_ROLE` (a role with Build); plain `http://` only.

Add `--cancel <reason>` to stop a hosted goal. The run takes the goal's
assignment over, so earlier holders are fenced out, cancels each hosted BBP task,
and returns only when every process the task started has been reaped and its
slot is free. It then marks the card's execution cancelled and retires the
assignment, so a later `run` on that state is refused. Running the cancel again
finishes an interrupted one and does nothing after a complete one. Set the BBP
configuration's `end_grace_secs` to bound how long a harness may run after its
turn ends (BBP's default is 60 s).

Proposed-change impact: `rusty_orch inspect --repo <checkout> --impact <request.json>`
joins Cargo consumers to Millboard ownership and complete RemindMe requirements.
See [Stage 1 usage and snapshot contract](docs/change-impact.md).
