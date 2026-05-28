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

### Desktop app (Tauri 2)

The desktop frontend lives in [`desktop/`](desktop) (its own workspace). It needs
the [Tauri 2 system prerequisites](https://tauri.app/start/prerequisites/)
(WebKitGTK on Linux), Node, and the Tauri CLI (`cargo install tauri-cli --version "^2"`):

```bash
cd desktop
npm install
cargo tauri dev      # builds the SolidJS frontend + launches the webview
# cargo tauri build  # produce a release bundle
```

All model calls stay on the Rust side; API keys live in the OS keychain, never in
the frontend.

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
| `RUSTYKEYS_PERMISSION_MODE` | `default` | `default`/`plan`/`accept_edits`/`read_only`/`restricted`/`bypass` |
| `RUSTYKEYS_ALLOWED_TOOLS` | *(unset)* | CSV allowlist for `restricted` mode |
| `RUSTYKEYS_ALLOW_BYPASS` | *(off)* | `1` required to enable `bypass` mode |
| `RUSTYKEYS_ISOLATION` | `none` | `none` (in-process) or `sandboxed` (OS sandbox for `bash`; Linux) |
| `RUSTYKEYS_CONTEXT_LIMIT` | `200000` | Model context window in tokens (drives compaction) |
| `RUSTYKEYS_COMPACT_MICRO` / `_SESSION` / `_FULL` | `0.80` / `0.90` / `0.95` | Compaction thresholds (fraction of the window) |
| `RUSTYKEYS_MAX_AGENT_DEPTH` | `3` | Subagent recursion bound |
| `RUSTYKEYS_IDLE_THRESHOLD` | `8` | Observations before idle consolidation |
| `RUSTYKEYS_EXPLORE` | *(off)* | `1` enables the opt-in divergent→converge `explore` tool (cost-gated) |
| `RUSTYKEYS_EXPLORE_BRANCHES` / `_TOP_K` | `5` / `2` | Divergent branch count `N` / converge top-`K` |

### REPL commands

`/verify` · `/mhir` · `/memory` · `/task` · `/plan` · `/explore` · `/mcp` · `/permissions` · `/entropy` · `/cost` · `/compact` · `/stats` · `/model` · `/config` · `/env` · `/doctor` · `/init` · `/diff` · `/commit` · `/branch` · `/review` · `/reflect` · `/sleep` · `/groom` · `/help` · `/quit`

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
| `mcp` | MCP client (manager/policy/namespacing/inspection) + `rmcp` transport (feature-gated) + server mode |
| `compose` | `Verifier`/`Check`, `FailureType` attribution, `CriteriaJudge`, evidence journal |
| `app` | `Session` (the centre) + the CLI |

The authoritative component map and crate DAG live in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §4-5.

The **desktop frontend** (Phase 15) lives in [`desktop/`](desktop) as its own
cargo workspace (Tauri 2 + SolidJS), deliberately excluded from the core
workspace so `cargo build --workspace` stays lean and WebKit-free. It never
calls a model directly — it talks to `Session` only through the canonical IPC
contract ([`docs/reference/ipc-contract.md`](docs/reference/ipc-contract.md)),
the same `rk://` event + `invoke` surface the gateway and ACP adapters mirror.

## Tool suite

Filesystem (`read_file`, `list_directory`, `write_file`, `edit_file`, `glob`,
`grep`), shell (`bash`, BashGuard-vetted), web (`web_fetch`/`web_search`,
opt-in + SSRF guard), the `agent` subagent (depth-bounded), Task State
(`set_task`/`complete_task`), and task management
(`task_create`/`get`/`list`/`update`/`stop`/`output`). Every call is
policy-vetted before dispatch; results carry a structured `ToolOutcome`.

## Implementation status

Phases 1–16 of the [roadmap](BACKLOG.md) are implemented. Every LLM-dependent
path is covered by a scripted `FakeLanguageModel`, so the whole system is
testable in CI without a live provider.

- **1 · Skeleton** — workspace, kernel loop, `Session`, CLI, error model, CI.
- **2 · Verify** — deterministic checks, fixed `FailureType` attribution, evidence journal, M-HIR, chaos/resilience tier.
- **3 · Memory** — short-term stream + long-term store (SQLite/FTS5), recall scoring, tiered consolidation, validation-gated skills + grooming.
- **4 · Task State + judge** — working-memory task + the semantic `CriteriaJudge` (`judge_unavailable` is never a silent pass).
- **5 · Embeddings** — semantic recall on SQLite (cosine + lexical fallback) via any OpenAI-compatible embed endpoint. *(DuckDB is a deferred at-scale backend.)*
- **6 · Tool suite** — the filesystem/shell/web/subagent/task tools above.
- **7 · Permission system** — permission modes (`ModePolicy`), the `SecurityCheck` suite + `security.jsonl`, and the channel-based `ApprovalGate`; blocks log a `tool_block` intervention.
- **7B · Capability isolation** — the `ToolExecutor` seam (`RUSTYKEYS_ISOLATION=none|sandboxed`); `sandboxed` runs `bash` inside an OS sandbox (bubblewrap/firejail) with network-deny + workspace-only FS, failing closed if no launcher is present.
- **8 · Token & context** — a line-item `TokenBudget` (system + recall + task + tool schemas + history) driving 3-tier compaction (micro drop / session summary / full reset), recall↔history de-dup, journaled `compaction` events, and `/cost` + `/compact`.
- **9 · Plan mode** — `enter_plan_mode`/`exit_plan_mode` tools over a shared `PlanController`; plan mode blocks writes/bash live, and an `exit_plan_mode` request awaits a human Proceed/Reject/Annotate (`/plan`) before the next turn may write. Plan approval is not an intervention. Includes the opt-in divergent→converge `explore` strategy (`RUSTYKEYS_EXPLORE=1`, `/explore`): fan out N framed children, mechanically score→cluster→top-K, then one critic synthesis (ADR-0032).
- **10 · H3 episode packages** — at `RUSTYKEYS_HARNESS_LEVEL=h3`, every turn writes a versioned 8-trace `EpisodePackage` to `episodes/<turn_id>.json`, projected from raw evidence by the `EpisodeAssembler` (ADR-0036); the 5-label outcome classifier, the `reproduce`/`attribute_failure`/`verification_report` tools + `reproduce_before_edit`/`verification_report_required` checks, the `CheckRegistry` running `checks.toml` (project + local precedence) into the verdict + `verification_trace`, and the controlled-visibility ablation eval-substrate (`app::eval`, ADR-0035): per-episode isolated workspaces, level-visibility, and evaluator-side adjudication at every level.

- **11 · Entropy auditor** — per-turn syntactic heuristics over `edit_file`/`write_file` (Residue, TestWeakening, StaleDocs, DependencyChurn, BoundaryViolation, TaskContradiction) with 0–3 severity; informational (doesn't flip `verified`), but sev≥2 TestWeakening/BoundaryViolation forces the H3 outcome to `unsafe_invalid`. Written to `.rustykeys/entropy.jsonl`; `/entropy` shows recent findings.

- **12 · MCP** — MCP client manager over the `McpClient` seam (`mcp.toml`, `mcp__server__tool` namespacing, `McpPolicy`, `McpToolFn`→`ToolOutcome`, tool-return inspection, reconnect; `/mcp`), the `rmcp`-backed stdio transport adapter + cargo-deny license gate (behind the `rmcp` feature), and MCP server mode exposing `Session::send()` as a `chat` tool (`rusty-keys --mcp`, behind the `mcp-server` feature). The SSE transport + auth/TLS is a documented follow-on on the same seam.

- **13 · Extended CLI** — `/stats` (turns/tool-calls/tokens/M-HIR/entropy), `/model`, `/config`, `/env`, `/doctor` (model/workspace/SQLite/MCP health), `/init` (`AGENT_GUIDE.md`), and git commands (`/diff`/`/branch` direct, `/commit`/`/review` via the agent).
- **14 · Web gateway** — an `axum` HTTP/SSE surface over `Session::send()` (`rusty-keys --gateway`, behind the `gateway` feature): `POST /chat`, `GET /stream` (named SSE frames), `/health`, `/verify`, `/evidence`, `/mhir`, `/entropy`; single + multi-session (idle-TTL + max-session eviction, `session_id`↔auth binding), bearer auth, and CORS.
- **15 · Desktop frontend** — a Tauri 2 + SolidJS + CodeMirror 6 + xterm.js desktop app ([`desktop/`](desktop), its own cargo workspace) that is a reactive rendering layer over `Session`. The Rust bridge registers one `#[tauri::command]` per name in the canonical IPC contract (`app::contract`) and emits the nine `rk://` events — including **live `rk://token` streaming** via the kernel's `stream_turn` and `rk://bash_output` to the terminal — with turn failures rendered through the boundary-error taxonomy; provider keys live only in the OS keychain. The UI is an AI-first layout (streaming session panel, xterm.js terminal, CM6 diff editor, composer with `@file`/`#memory`/`/command` + approval/plan gates, harness dashboard, settings/themes). A headless Tauri IPC smoke test (mock runtime) plus the frontend build gate it in CI.
- **16 · ACP** — an Agent Client Protocol server (`rusty-keys --acp`): newline-delimited JSON-RPC 2.0 over stdio exposing the `Session` to editors. Handshake (`initialize`/`authenticate`), `session/new`/`prompt`/`cancel` → `Session::send()` with `session/update` notifications, and a `session/request_permission` round-trip bound to the Phase-7 `ApprovalGate` (a denied write holds the boundary). Client fs/terminal capability shims are a documented follow-on.

Remaining follow-ons: the MCP SSE transport (+auth/TLS), the gateway readiness
probe, ACP fs/terminal shims, and a desktop git-status tab.

## Building & testing

```bash
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

MSRV **1.88** (aisdk 0.5.2 uses let-chains). CI runs `{stable, 1.88}` ×
build + fmt + clippy + test + feature builds — see
[`docs/dev/coding-standards.md`](docs/dev/coding-standards.md). The desktop
workspace is gated by a dedicated CI job (frontend build + `fmt`/`clippy` +
the Tauri IPC smoke test).

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the system view: component map, crate DAG, concurrency, topologies, faithfulness map. **Read this first.**
- [`docs/`](docs) — PRDs ([`prd/`](docs/prd)), decision records ([`adr/`](docs/adr)), the on-disk [data model](docs/architecture/data-model.md), and the [configuration reference](docs/reference/configuration.md).
- [`BACKLOG.md`](BACKLOG.md) — the development roadmap.
