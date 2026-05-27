# PRD 03 — Feed

## Responsibility

The feed layer is the **Observe + Orient** half of the OODA loop. It supplies
the kernel with everything it needs to reason well:

- **Tools**: what the agent can do (`#[tool]` registry)
- **Context**: system prompt + oriented working memory (recall + task prompt)
- **Memory**: short-term stream capture, long-term graph, Task State

Feed does not execute — it prepares. The kernel executes.

## Tools

### `#[tool]` proc macro

aisdk's `#[tool]` macro annotates a Rust function and generates the JSON schema
the model needs, eliminating the manual schema authorship that Keystone required:

```rust
#[tool]
/// Read a file from the workspace. Path must be within the workspace root.
pub async fn read_file(path: String) -> Result<String, ToolError> {
    tokio::fs::read_to_string(&path).await.map_err(ToolError::from)
}
```

The macro derives: tool name (function name), description (doc comment),
parameter schema (function signature), return handling.

### `ToolRegistry`

Tools are registered at startup and passed to the kernel. The registry handles
dispatch (with policy vetting — see PRD 02) and exposes the tool list to aisdk.

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolFn>>,
}
```

MCP server tools are registered alongside built-ins with namespaced names
(`mcp__<server>__<tool>`) — see PRD 07.

## Built-in tool suite

The full set of tools registered at `Session::new()`. All are policy-vetted
before dispatch.

### Filesystem tools

| Tool | Description |
|---|---|
| `read_file(path)` | Read a file; returns content as string |
| `write_file(path, content)` | Create or overwrite a file; creates parent dirs |
| `edit_file(path, old_string, new_string)` | Targeted replacement; fails if 0 or 2+ matches |
| `list_directory(path)` | List directory contents |
| `glob(pattern)` | Pattern-matched file listing; workspace-relative paths |
| `grep(pattern, path?, recursive?)` | Content search; returns `path:line: content`, capped at 200 |

`edit_file` enforces a read-before-edit invariant in AcceptEdits mode: the policy
checks that `read_file` was called on the same path in the current episode.

### Shell tool

```rust
#[tool]
/// Execute a shell command in the workspace.
pub async fn bash(command: String, timeout_ms: Option<u64>) -> Result<String, ToolError>
```

- Runs via `tokio::process::Command`; captures stdout + stderr combined
- Default timeout: 30s; returns `"TIMEOUT: …"` on expiry
- Non-zero exit code: `"ERROR (exit {code}): …"`
- Vetted by `BashGuard` (see PRD 02) before execution

Background variant:

```rust
#[tool]
/// Spawn a long-running process; returns a handle for logs/stop.
pub async fn bash_background(command: String) -> Result<String, ToolError>

#[tool]
/// Get accumulated output from a background process by handle.
pub async fn bash_logs(handle: String) -> Result<String, ToolError>

#[tool]
/// List all running background processes.
pub async fn bash_list() -> Result<String, ToolError>

#[tool]
/// Stop a background process by handle.
pub async fn bash_kill(handle: String) -> Result<String, ToolError>
```

### Web tools

Blocked by default; enabled by `RUSTYKEYS_ALLOW_WEB=1`.

```rust
#[tool]
/// Fetch a URL and return its content as plain text (HTML stripped).
pub async fn web_fetch(url: String, prompt: Option<String>) -> Result<String, ToolError>

#[tool]
/// Search the web and return structured results (title, URL, snippet).
pub async fn web_search(query: String, num_results: Option<usize>) -> Result<String, ToolError>
```

`web_fetch` uses `reqwest` + HTML stripping; content capped at ~50k chars.
`web_search` backend configured via `RUSTYKEYS_SEARCH_PROVIDER`
(`brave` | `serper` | `duckduckgo`).

### Agent tool (subagent spawning)

```rust
#[tool]
/// Spawn a subagent Session to complete a focused subtask.
pub async fn agent(task: String, tools: Option<Vec<String>>) -> Result<String, ToolError>
```

Creates a child `Session` with an isolated history and the specified tool subset.
Inherits `Config` and `WorkspacePolicy` from the parent. `AgentDepthPolicy`
prevents recursion beyond `RUSTYKEYS_MAX_AGENT_DEPTH` (default 3).
The child's episode is recorded as a nested entry in `EvidenceJournal`.

### Task management tools

Manage background operations the agent initiates (subagent runs, long bash
processes). Distinct from `TaskState` (the goal/criteria working memory).

```rust
#[tool]
/// Create a background task; returns its ID.
pub async fn task_create(description: String) -> Result<String, ToolError>

#[tool]
/// Get status and recent output of a task by ID.
pub async fn task_get(id: String) -> Result<String, ToolError>

#[tool]
/// List all tasks with their current status.
pub async fn task_list() -> Result<String, ToolError>

#[tool]
/// Append a note or status update to a task.
pub async fn task_update(id: String, note: String) -> Result<String, ToolError>

#[tool]
/// Stop a running task by ID.
pub async fn task_stop(id: String) -> Result<String, ToolError>

#[tool]
/// Retrieve the full output of a completed task.
pub async fn task_output(id: String) -> Result<String, ToolError>
```

`TaskStore` holds an in-session registry; tasks persist for the session lifetime
but not across sessions. `task_stop` sends a `CancellationToken` to the task's
`tokio` handle.

### Plan mode tools

```rust
#[tool]
/// Enter plan mode: writes and bash are blocked until the user approves.
pub fn enter_plan_mode() -> Result<String, ToolError>

#[tool]
/// Exit plan mode and request user approval before execution resumes.
pub fn exit_plan_mode() -> Result<String, ToolError>
```

See PRD 06 for the plan mode lifecycle.

### H3 verification tools

Active only when `RUSTYKEYS_HARNESS_LEVEL=h3`. See PRD 05 for full design.

```rust
#[tool]
/// Run a deterministic check and record observed vs expected output.
pub async fn reproduce(check_name: String) -> Result<String, ToolError>

#[tool]
/// Record a structured failure attribution before making any edit.
pub async fn attribute_failure(
    observed: String, expected: String,
    failure_type: String, layer: String,
    evidence: String, next_action: String,
) -> Result<String, ToolError>

#[tool]
/// Generate and record a verification report linking requirements to evidence.
pub async fn verification_report(
    requirements: Vec<RequirementEvidence>,
    limits: String,
) -> Result<String, ToolError>
```

### Memory tools (registered from memory module)

```rust
#[tool]
/// Set the active task goal and success criteria.
pub async fn set_task(goal: String, success_criteria: Vec<String>) -> Result<String, ToolError>

#[tool]
/// Mark the active task as complete.
pub async fn complete_task(summary: String) -> Result<String, ToolError>
```

## Memory

Memory is the three-tier cognitive architecture:

```
Short-term stream  →  [consolidation]  →  Long-term graph
        ↑                                        ↓
   (Observe: capture)                    (Orient: recall)
        ↑                                        ↓
   Task State (working memory: goal + success criteria)
```

### Short-term stream

An append-only log of observations: user messages, agent replies, tool events,
verification verdicts, task changes. Every event in the system passes through
`Stream::append()`.

```rust
pub trait Stream: Send + Sync {
    async fn append(&self, obs: &Observation) -> Result<()>;
    async fn recent(&self, n: usize) -> Result<Vec<Observation>>;
    async fn since(&self, ts: f64) -> Result<Vec<Observation>>;
}
```

`Observation` fields: `ts`, `role`, `kind`, `content`.

Default implementation: `SqliteStream` via `rusqlite` (append-optimised,
OLTP-pattern).

### Long-term graph

Consolidated memories: facts, summaries, skills, entities — with typed edges
between them. Recall queries this store to orient the kernel.

```rust
pub trait Store: Send + Sync {
    async fn upsert(&self, memory: &Memory) -> Result<()>;
    async fn candidates(&self, query: &str, embed: Option<&[f32]>, k: usize)
        -> Result<Vec<(Memory, f32)>>;
    async fn neighbors(&self, title: &str) -> Result<Vec<Memory>>;
    async fn prune(&self, older_than: f64, importance_below: f32) -> Result<usize>;
    async fn remove(&self, title: &str) -> Result<()>;
}
```

`candidates()` returns `(memory, relevance)` — each backend uses its best
strategy (FTS5 lexical for SQLite, `list_cosine_similarity` for DuckDB).

Skills (`type = "skill"`) are exempt from `prune()`. They accumulate until
grooming (refine / merge / split) consolidates them.

### Recall

Recall assembles the Orient layer: score candidates by relevance + recency +
importance, take top-k, expand 1-hop via `neighbors()`.

```rust
pub async fn recall(
    &self,
    history: &[Message],
    window: usize,
    k: usize,
) -> Result<String>
```

The query is built from the last `window` turns (not just the latest message)
— a follow-up like "do that again" has no retrieval signal alone; recent turns
carry the topic.

When `RUSTYKEYS_EMBED_MODEL` is set, recall is semantic (embedding cosine).
Without it, recall falls back to FTS5 lexical search — the system runs on a
chat key alone.

### Consolidation

Distillation of short-term → long-term runs at three tempos:

| Tempo | Trigger | Behaviour |
|---|---|---|
| idle | after each turn if `len(recent) >= idle_threshold` | extract facts/summaries/skills from recent observations |
| sleep | session end | deeper pass; decay + prune non-skills |
| explicit | `/reflect` or `/sleep` command | user-triggered |

Each consolidation is an async aisdk call with a structured prompt requesting
JSON output (`{"memories": [...]}`). `serde_json` deserialises the result; the
store is updated accordingly. The consolidation changelog is appended to the
evidence journal (ADR-015).

Verification signals (`kind = "verification"`) in the stream are treated as
high-value consolidation input: UNVERIFIED outcomes generate a high-importance
skill (the lesson); VERIFIED outcomes reinforce the skill used.

### Skill grooming

Once the skill count exceeds `RUSTYKEYS_SKILL_GROOM_THRESHOLD`, a grooming pass
asks the model to propose `refine`, `merge`, and `split` operations over the skill
set. Merges and splits supersede originals via `Store::remove()`. Grooming runs
on `/groom` (forced) and during `sleep`.

### Task State

`TaskState` is the working-memory tier: a single current goal + success criteria.

```rust
pub struct TaskState {
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub status: TaskStatus,   // Idle | Active | Done
    pub updated_ts: f64,
}
```

Persisted to `.rustykeys/task.json`. The agent maintains it via `set_task` and
`complete_task` tools — no extra per-turn LLM call. The harness injects the
active task into the system prompt (drift prevention) and anchors recall on the
goal.

## Context assembly

Before each kernel run, the feed layer assembles `extra_context`:

```rust
pub async fn orient(&self, history: &[Message]) -> String {
    let task_prompt = self.task_store.render();
    let recall = self.recall(history, self.config.recall_window, self.config.recall_k).await?;
    [task_prompt, recall].iter().filter(|s| !s.is_empty()).join("\n\n")
}
```

This string is injected between the system prompt and the first user message —
the kernel receives an oriented view of the conversation without knowing where
the context came from.

## Seams

- **Embedding model**: set `RUSTYKEYS_EMBED_MODEL` to any aisdk embed string for
  semantic recall. Absent → lexical fallback.
- **DuckDB backend**: `RUSTYKEYS_LONG_TERM_BACKEND=duckdb` for native vector
  search at scale (Phase 5).
- **MCP tools**: external MCP server tools registered in `ToolRegistry` with
  `mcp__<server>__<tool>` namespacing — see PRD 07.
- **Hierarchical rollups**: multi-cadence temporal consolidation (hourly/daily/
  weekly summaries-of-summaries). Tracked in BACKLOG.
- **Auto-maintained focus**: infer/refresh Task State automatically for turns
  where the agent doesn't call `set_task`. Additive to agent-driven approach.
- **Tool result classification**: explicit `ToolResultClassifier` (ok / blocked /
  error / truncated / timeout) for richer attribution — currently inferred from
  result string prefix.
