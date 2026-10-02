# rusty_orch

A small orchestrator that lets Claude, ChatGPT (Codex), Gemini, and local models (Ollama/Hermes) collaborate on development research and tasks without a human relaying context between them. Agents never message each other: they read and write a shared, append-only blackboard and a git repo, and a deterministic dispatcher hands each one a task card.

## Status
Experimental. Domain core (`orch-core`), an in-memory dispatcher (`orch-dispatch`), a shared CLI-adapter core (`orch-cli`), and two real adapters (`orch-ollama` for `Agent::Local`, `orch-codex` for `Agent::Codex`, read-only); no persistence yet. Owner: @baileyrd.

## Getting started
```powershell
# from the rusty_mill workspace root
cargo test -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex --all-features
ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-ollama --example research -- "How does Plan::start prevent self-review?"
ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-codex --example research_review -- "How does Plan::start prevent self-review?"
```

## Layout
| Path | What |
| ---- | ---- |
| `crates/orch-core` | Pure domain: goal contracts, task cards + `Plan`, blackboard. No I/O, no dependencies. |
| `crates/orch-dispatch` | Application layer: `AgentRunner` port, validated role → agent routing, sequential loop over `Plan` metered by a caller-owned `Ledger`. Depends only on `orch-core`. `FakeAgent` behind the `fake` feature. |
| `crates/orch-cli` | Shared CLI-adapter core: `CommandRunner`/`StdCommand` seam (hard deadline, group kill, env scrub), strict JSON `parse`, prompt `render` with a per-adapter footer, `fake` test doubles. |
| `crates/orch-ollama` | `Agent::Local` over `ollama run --format json`: thin adapter on `orch-cli`. `research` example runs one card end to end. |
| `crates/orch-codex` | `Agent::Codex` over `codex exec --sandbox read-only`, schema-constrained reply via the last-message file, `OPENAI_API_KEY` scrubbed. `research_review` example: Codex researches, a local model reviews. |

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) and [docs/adr/](./docs/adr/).

## Development
```powershell
cargo test -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex --all-features
cargo fmt -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex -- --check
cargo clippy -p orch-core -p orch-dispatch -p orch-cli -p orch-ollama -p orch-codex --all-targets --all-features -- -D warnings
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md). Agents working in this repo: read [AGENTS.md](./AGENTS.md) first.

## Security
See [SECURITY.md](./SECURITY.md).

## License
Internal — not for external distribution.
