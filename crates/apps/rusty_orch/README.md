# rusty_orch

A small orchestrator that lets Claude, ChatGPT (Codex), Gemini, and local models (Ollama/Hermes) collaborate on development research and tasks without a human relaying context between them. Agents never message each other: they read and write a shared, append-only blackboard and a git repo, and a deterministic dispatcher hands each one a task card.

## Status
Experimental. Domain core (`orch-core`) and an in-memory dispatcher (`orch-dispatch`); no real agent adapters or persistence yet. Owner: @baileyrd.

## Getting started
```powershell
# from the rusty_mill workspace root
cargo test -p orch-core -p orch-dispatch --all-features
```

## Layout
| Path | What |
| ---- | ---- |
| `crates/orch-core` | Pure domain: goal contracts, task cards + `Plan`, blackboard. No I/O, no dependencies. |
| `crates/orch-dispatch` | Application layer: `AgentRunner` port, validated role → agent routing, sequential loop over `Plan` metered by a caller-owned `Ledger`. Depends only on `orch-core`. `FakeAgent` behind the `fake` feature. |

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) and [docs/adr/](./docs/adr/).

## Development
```powershell
cargo test -p orch-core -p orch-dispatch --all-features
cargo fmt -p orch-core -p orch-dispatch -- --check
cargo clippy -p orch-core -p orch-dispatch --all-targets --all-features -- -D warnings
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md). Agents working in this repo: read [AGENTS.md](./AGENTS.md) first.

## Security
See [SECURITY.md](./SECURITY.md).

## License
Internal — not for external distribution.
