# Rusty Keys

An AI-native application skeleton in Rust. The model's agent loop is the kernel;
the application is the harness built around it.

Rusty Keys is the Rust successor to [Keystone](https://github.com/baileyrd/Keystone),
carrying forward its harness philosophy — constrain, feed, observe, compose — with
a Rust implementation that makes the architecture's natural properties (async,
type-safe, zero-overhead policy enforcement) first-class rather than aspirational.

## Why Rust

The harness is a runtime substrate — the layer that mediates every model action,
enforces policy on every tool call, and persists every observation. Those
responsibilities are a better fit for a systems language than for Python: the
constraints are not LLM-bound, they are execution-bound. The LLM calls themselves
(the slow part) are async I/O regardless of language; the harness layers that
wrap them benefit from Rust's ownership model, zero-cost abstractions, and the
native async runtime tokio provides.

## LLM Provider

Powered by [aisdk](https://aisdk.rs) — a provider-agnostic Rust LLM library
covering 70+ providers (Anthropic, OpenAI, Google, Ollama, OpenRouter, …) with
native async streaming and a `#[tool]` proc macro for type-safe tool registration.
The kernel drives aisdk's `LanguageModelRequest` loop and enforces policy from
inside each tool's dispatch (see [`docs/spike/01-aisdk-tool-seam.md`](docs/spike/01-aisdk-tool-seam.md)).

## Quick start

```bash
# Single-shot (defaults to a local ollama endpoint, http://localhost:11434/v1):
RUSTYKEYS_MODEL=llama3.1 cargo run -p rk-app -- "list the files in . and read Cargo.toml"

# Interactive REPL (no prompt argument):
RUSTYKEYS_MODEL=llama3.1 cargo run -p rk-app
```

The binary is `rusty-keys` (the `rk-app` crate). All state is local under
`.rustykeys/` in the workspace root.

### Configuration (environment)

| Variable | Default | Purpose |
|---|---|---|
| `RUSTYKEYS_MODEL` | *(required)* | Model name sent to the OpenAI-compatible endpoint |
| `RUSTYKEYS_BASE_URL` | `http://localhost:11434/v1` | Provider endpoint (ollama by default) |
| `RUSTYKEYS_API_KEY` | `ollama` | Provider API key |
| `RUSTYKEYS_WORKSPACE` | cwd | Workspace root + policy boundary |
| `RUSTYKEYS_HARNESS_LEVEL` | `h1` | Maturity level `h0`–`h3` |
| `RUSTYKEYS_EMBED_MODEL` | *(unset)* | Embedding model ⇒ semantic recall (else lexical) |
| `RUSTYKEYS_ALLOW_WEB` | *(off)* | `1` enables the web tools (SSRF-guarded) |
| `RUSTYKEYS_MAX_AGENT_DEPTH` | `3` | Subagent recursion bound |
| `RUSTYKEYS_IDLE_THRESHOLD` | `8` | Observations before idle consolidation |

### REPL commands

`/verify` · `/mhir` · `/memory` · `/task` · `/reflect` · `/sleep` · `/groom` · `/help` · `/quit`

## Architecture

Four harness verbs wrap the aisdk agent kernel:

```
┌─────────────────────────────────────────────────┐
│                    Session                       │
│  ┌──────────┐  ┌──────┐  ┌───────┐  ┌────────┐ │
│  │constrain │  │ feed │  │observe│  │compose │ │
│  └────┬─────┘  └──┬───┘  └───┬───┘  └───┬────┘ │
│       │           │          │           │      │
│  ┌────▼───────────▼──────────▼───────────▼────┐ │
│  │              aisdk Kernel                   │ │
│  └────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

**Eight crates** (DAG: `config ← observe ← constrain ← feed`; `kernel → constrain`;
`compose → feed`; `app → all`):

| Crate | Responsibility |
|---|---|
| `config` | `RUSTYKEYS_*` resolution (leaf) |
| `observe` | `ToolOutcome`, `Tracer`/`Episode`, redaction, `InterventionLogger` (M-HIR) |
| `constrain` | `Policy` + `ToolDispatch`, `WorkspacePolicy`, `BashGuard` |
| `feed` | tool registry + suite, memory (`Stream`/`Store`/recall/consolidation), `TaskStore`, `SessionFactory` |
| `kernel` | the aisdk loop + the policy-vetting bridge |
| `mcp` | MCP seam (stub; rmcp client/server land in Phase 12) |
| `compose` | `Verifier`/`Check`, `FailureType` attribution, `CriteriaJudge`, evidence journal |
| `app` | `Session` (the centre) + the CLI |

The authoritative component map and crate DAG live in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §4-5.

## Tool suite

Filesystem (`read_file`, `list_directory`, `write_file`, `edit_file`, `glob`,
`grep`), shell (`bash`, BashGuard-vetted), web (`web_fetch`/`web_search`,
opt-in + SSRF guard), the `agent` subagent (depth-bounded), Task State
(`set_task`/`complete_task`), and task management
(`task_create`/`get`/`list`/`update`/`stop`/`output`). Every call is
policy-vetted before dispatch; results carry a structured `ToolOutcome`.

## Implementation status

Phases 1–6 of the [roadmap](BACKLOG.md) are implemented. Every LLM-dependent
path is covered by a scripted `FakeLanguageModel`, so the whole system is
testable in CI without a live provider.

- **1 · Skeleton** — workspace, kernel loop, `Session`, CLI, error model, CI.
- **2 · Verify** — deterministic checks, fixed `FailureType` attribution, evidence journal, M-HIR, chaos/resilience tier.
- **3 · Memory** — short-term stream + long-term store (SQLite/FTS5), recall scoring, tiered consolidation, validation-gated skills + grooming.
- **4 · Task State + judge** — working-memory task + the semantic `CriteriaJudge` (`judge_unavailable` is never a silent pass).
- **5 · Embeddings** — semantic recall on SQLite (cosine + lexical fallback) via any OpenAI-compatible embed endpoint. *(DuckDB is a deferred at-scale backend.)*
- **6 · Tool suite** — the filesystem/shell/web/subagent/task tools above.

Remaining: permission modes (7), capability isolation (7B), token/context
management (8), plan mode (9), H3 episode packages (10), entropy auditor (11),
MCP (12), extended CLI (13), web gateway (14), desktop frontend (15).

## Building & testing

```bash
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

MSRV **1.88** (aisdk 0.5.2 uses let-chains). CI runs `{stable, 1.88}` ×
build + fmt + clippy + test + feature builds — see
[`docs/dev/coding-standards.md`](docs/dev/coding-standards.md).

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the system view: component map, crate DAG, concurrency, topologies, faithfulness map. **Read this first.**
- [`docs/`](docs) — PRDs ([`prd/`](docs/prd)), decision records ([`adr/`](docs/adr)), the on-disk [data model](docs/architecture/data-model.md), and the [configuration reference](docs/reference/configuration.md).
- [`BACKLOG.md`](BACKLOG.md) — the development roadmap.
</content>
</invoke>
