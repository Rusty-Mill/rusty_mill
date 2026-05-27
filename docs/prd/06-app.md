# PRD 06 — App (Session + CLI)

## Responsibility

The app layer has two parts:

- **`Session`**: the transport-agnostic AI loop. Owns the full turn cycle —
  observe → orient → kernel → compose — and exposes a single `send()` method.
  This is the centre of the system.
- **CLI adapter**: a thin `tokio::main` REPL over `Session`. The only part of
  the codebase that touches `stdin`/`stdout`.

These two are deliberately separate. Any future gateway (web, desktop, API) is a
different adapter over the same `Session`.

## Session

```rust
pub struct Session {
    config:        Config,
    kernel:        Kernel,
    registry:      ToolRegistry,
    policy:        Box<dyn Policy>,
    memory:        Memory,
    tracer:        Tracer,
    verifier:      Verifier,
    judge:         CriteriaJudge,
    journal:       EvidenceJournal,
    interventions: InterventionLogger,
    history:       Vec<Message>,
    last_report:   Option<VerificationReport>,
}
```

### `send()`

```rust
impl Session {
    pub async fn send(&mut self, message: &str) -> Result<(String, VerificationReport)>;
}
```

Full turn cycle:

```
1. Detect unverified_followup (if last_report is Some and !verified)
2. memory.observe(user message)
3. history.push(user message)
4. context = memory.orient(&history).await
5. tracer.start_episode()
6. reply = kernel.run(&history, &registry, &policy, context, &mut tracer).await
7. history.push(assistant reply)
8. memory.observe(reply)

Concurrent post-turn:
9a. report = verifier.verify_with_judge(&reply, tracer.episode(), &judge).await
9b. idle_stats = memory.consolidate(Idle).await

10. memory.observe(report.as_observation())
11. journal.record_turn(&reply, tracer.episode(), &report)
12. last_report = Some(report.clone())
13. return (reply, report)
```

Steps 9a and 9b run via `tokio::join!` — the criteria judge and idle
consolidation overlap while the reply is already in the caller's hands.

### Session on its own task

For the thread-architecture variant (web gateway, or a CLI that wants a
responsive input loop), `Session` can be moved onto a tokio task with an
`mpsc` channel pair:

```rust
let (tx_in, rx_in) = mpsc::channel::<String>(1);
let (tx_out, rx_out) = mpsc::channel::<(String, VerificationReport)>(1);

tokio::spawn(async move {
    let mut session = Session::new(config).await?;
    while let Some(msg) = rx_in.recv().await {
        let result = session.send(&msg).await?;
        tx_out.send(result).await?;
    }
    session.shutdown().await?;
    Ok::<_, Error>(())
});
```

The CLI then sends to `tx_in` and reads from `rx_out` — the input prompt is
available immediately after the user hits enter, before the kernel has finished.
The channel is sized 1 to preserve the turn-by-turn ordering guarantee.

### `shutdown()`

```rust
impl Session {
    pub async fn shutdown(&mut self) -> Result<()>;
}
```

Called on exit. Runs sleep-tempo consolidation, skill grooming, and flushes the
evidence journal. Mirrors Keystone's end-of-session behaviour.

## CLI adapter

The CLI is a `tokio::main` function and nothing more. All AI logic lives in
`Session`. All slash commands call `Session` or query its components directly.

### Commands

| Command | Action | Intervention logged |
|---|---|---|
| `/task [goal]` | Show or set active task | `task_override` if task already active |
| `/memory` | Print long-term memory snapshot | — |
| `/reflect` | Explicit consolidation | `manual_reflect` |
| `/sleep` | Deep consolidation | `manual_reflect` |
| `/groom` | Skill grooming pass | `manual_groom` |
| `/verify` | Show last verification report | `manual_verify` |
| `/evidence` | Show recent evidence journal | — |
| `/mhir` | Show intervention rate and breakdown | — |
| `exit` / `quit` | Shutdown and exit | — |

### `/mhir` output

```
M-HIR: 3 interventions / 12 turns = 25.0%
  unverified_followup: 2
  manual_verify: 1
```

### Startup banner

```
Rusty Keys :: model=anthropic/claude-opus-4-7 :: workspace=/path/to/ws
memory :: short-term=sqlite(memory.db) :: long-term=sqlite(memory.db) :: recall=lexical
Commands: /task [goal], /memory, /reflect, /sleep, /groom, /verify, /evidence, /mhir, exit
```

## Config

All configuration resolved from environment variables at startup.

| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_MODEL` | `anthropic/claude-opus-4-7` | Any aisdk model string |
| `RUSTYKEYS_WORKSPACE` | `.` | Workspace root (policy boundary) |
| `RUSTYKEYS_TRACE` | `1` | Enable trace logging to stderr |
| `RUSTYKEYS_VERIFY` | `1` | Enable verification + evidence journal |
| `RUSTYKEYS_MAX_STEPS` | `10` | Kernel loop step limit |
| `RUSTYKEYS_SHORT_TERM_BACKEND` | `sqlite` | `sqlite` |
| `RUSTYKEYS_LONG_TERM_BACKEND` | `sqlite` | `sqlite` or `duckdb` |
| `RUSTYKEYS_EMBED_MODEL` | _(none)_ | aisdk embed model string; absent = lexical recall |
| `RUSTYKEYS_RECALL_K` | `6` | Top-k memories to retrieve |
| `RUSTYKEYS_RECALL_WINDOW` | `6` | Recent turns used as recall query |
| `RUSTYKEYS_IDLE_THRESHOLD` | `8` | Observations before idle consolidation fires |
| `RUSTYKEYS_SKILL_GROOM_THRESHOLD` | `12` | Skills before grooming fires on sleep |
| `RUSTYKEYS_EVIDENCE_LOG` | `.rustykeys/evidence.jsonl` | Evidence journal path |
| `RUSTYKEYS_INTERVENTIONS_LOG` | `.rustykeys/interventions.jsonl` | Intervention log path |
| `RUSTYKEYS_TASK_FILE` | `.rustykeys/task.json` | Task State persistence path |

## Cargo workspace layout

```
rusty-keys/
├── Cargo.toml               # workspace
├── crates/
│   ├── kernel/              # Kernel struct, aisdk integration
│   ├── constrain/           # Policy trait + WorkspacePolicy
│   ├── feed/                # ToolRegistry, Memory, TaskStore, context assembly
│   │   └── memory/          # Stream, Store, consolidate, refine, recall
│   ├── observe/             # Tracer, InterventionLogger
│   ├── compose/             # Verifier, checks, CriteriaJudge, EvidenceJournal
│   ├── app/                 # Session, CLI
│   └── config/              # Config
└── BACKLOG.md
```

Crate boundaries enforce the architectural separation: `kernel` cannot import
`feed` or `compose`; `observe` cannot import `compose`; `app` imports everything.
The dependency graph is a DAG.

## Seams

- **Web gateway**: an `axum` route that holds a `Session` in `Arc<Mutex<Session>>`
  (or per-connection `Session`) and calls `session.send()`. The Vercel AI SDK UI
  wire protocol is compatible with aisdk's streaming output.
- **Streaming CLI**: replace `session.send()` return with a stream handle;
  print tokens as they arrive. Requires `stream_text()` (Phase 5).
- **Per-request sessions**: for a multi-user server, `Session::new()` is called
  per connection; the `Config` is shared, the session state is not.
