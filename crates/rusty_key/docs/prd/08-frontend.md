# PRD 08 — Frontend (Desktop)

## Responsibility

The desktop frontend is the primary interactive surface for Rusty Keys. It
communicates with the Rust backend exclusively via Tauri IPC — it makes no
AI SDK calls, holds no API keys, and performs no model inference.

**The frontend is a reactive rendering layer over the harness.** Its job is to
surface the outputs of `Session::send()` — verification reports, tool traces,
episode packages, entropy audits, M-HIR metrics — in a way that makes the
agent's behaviour transparent and auditable.

## Tech stack

| Layer | Choice | Rationale |
|---|---|---|
| Desktop shell | Tauri 2 | Rust backend, OS keychain, native PTY, small bundle |
| UI framework | SolidJS | Fine-grained signal reactivity; no virtual DOM overhead |
| Editor | CodeMirror 6 | Diff-first file view with hunk accept/reject |
| Terminal | xterm.js + WebGL addon | Native PTY output from `bash` tool |
| Styling | Tailwind v4 | Framework-agnostic utility classes |
| Build | Vite + `@solidjs/vite-plugin` | |
| State | SolidJS `createSignal` / `createStore` | No external state library |

No JS-side AI SDK. All API keys stored in the OS keychain via Tauri's `keyring`
crate — they never exist in JS memory longer than the `invoke` call that passes
them to Rust.

## Tauri IPC bridge

The only channel between frontend and Rust backend.

### Commands (frontend → Rust, request/response)

```ts
invoke<TurnResult>('session_send', { message, attachments })
invoke<void>('session_command', { command })         // /compact, /reflect, etc.
invoke<VerificationReport>('session_last_report')
invoke<MhirReport>('session_mhir')
invoke<Config>('session_config')
invoke<AppConfig>('config_set', { key, value })
invoke<void>('secrets_set', { provider, key })        // stores in OS keychain
invoke<string>('secrets_get', { provider })           // retrieves from keychain
invoke<void>('secrets_delete', { provider })
invoke<void>('approval_respond', { approved, always })
invoke<MemorySnapshot>('session_memory_snapshot')
invoke<EvidenceEntry[]>('session_evidence_recent', { n })
invoke<EntropyAudit[]>('session_entropy_history')
invoke<TokenBudget>('session_token_budget')
invoke<McpServer[]>('mcp_servers_list')
invoke<void>('mcp_server_add', { server })
invoke<void>('mcp_server_remove', { name })
invoke<McpTestResult>('mcp_server_test', { name })
invoke<string[]>('fs_list_workspace')               // for @file picker
invoke<MemoryEntry[]>('session_memory_search', { q }) // for #memory picker
invoke<string[]>('session_commands_list')            // for /command palette
```

### Events (Rust → frontend, pushed)

The `rk://` event set is **canonical in [PRD 06's event table](06-app.md#tauri-event-bridge--canonical-rk-event-table)** — this list mirrors it
(the gateway SSE channel mirrors the same names). The earlier draft of this PRD
used `rk://turn_start` in the Composer lock logic below without listing it among
the events; it is now in the canonical table and listed here:

```ts
listen<{ turn_id: string }>('rk://turn_start', handler) // turn began; lock composer
listen<string>('rk://token', handler)                // streaming token chunk
listen<ToolEvent>('rk://tool_event', handler)        // tool call fired
listen<TurnResult>('rk://turn_complete', handler)    // full turn result
listen<ApprovalRequest>('rk://approval_request', handler) // gate triggered
listen<string>('rk://plan_exit', handler)            // plan text to confirm
listen<EntropyAudit>('rk://entropy', handler)        // post-turn entropy
listen<string>('rk://bash_output', handler)          // bash stdout/stderr chunk
listen<ConsolidationStats>('rk://consolidation', handler) // consolidation done
```

### Errors (`invoke` rejection handling)

A failing turn does not arrive as an `rk://` event — it surfaces as a **rejected
`invoke` promise**. The rejection payload is the boundary error taxonomy from
[PRD 06](06-app.md#boundary-error-taxonomy): `{ kind, message }` where `kind` ∈
`provider_error | timeout | rate_limited | auth_error | policy_block | internal`.
The frontend catches the rejection, renders it uniformly (e.g. a dismissible
banner over the composer), and unlocks the composer — because a failed turn never
fires `rk://turn_complete`, the `catch` is the only path that clears the lock on
failure:

```ts
invoke('session_send', { message, attachments }).catch(err => {
  // err = { kind, message } from the boundary error taxonomy (PRD 06)
  showError(err)
  setLocked(false)   // failed turns don't emit rk://turn_complete
})
```

A `policy_block` rejection is distinct from the per-tool approval-gate flow
(`rk://approval_request`): the gate is an in-turn interaction, whereas a
`policy_block` rejection ends the turn.

## AI-first layout

The session panel is primary; context surfaces are secondary and respond to
agent actions automatically.

```
┌─────────────────────────────────────────────────────┐
│  Header: model | mode | tokens | M-HIR | task state │
├──────────────────────┬──────────────────────────────┤
│                      │                              │
│   SESSION PANEL      │   CONTEXT PANEL              │
│   (primary, left)    │   (secondary, right)          │
│                      │                              │
│   Conversation       │   Terminal  ← auto on bash   │
│   Episode trace      │   Editor    ← auto on edit   │
│   Verification       │   Git       ← auto on commit │
│   Task banner        │   Memory    ← auto on recall │
│                      │   Web       ← auto on fetch  │
├──────────────────────┴──────────────────────────────┤
│  Composer: input | @file | #memory | /command       │
└─────────────────────────────────────────────────────┘
```

Context panel auto-focus rule (driven by `rk://tool_event`):

| Tool fired | Tab opened |
|---|---|
| `bash`, `bash_background` | Terminal |
| `read_file`, `edit_file`, `write_file` | Editor |
| `glob`, `list_directory` | Editor (file tree) |
| `web_fetch`, `web_search` | Web |
| `rk://consolidation` event | Memory |
| Git-touching bash commands | Git |

A pin toggle prevents auto-switching when the human is using a tab.

## Components

### Header bar

Persistent. Updated after each `rk://turn_complete`.

- Model name + provider icon
- Permission mode badge (Default / Plan / AcceptEdits / ReadOnly)
- Token budget: `N / limit (X%)`; colour shifts yellow at 80%, red at 90%
- M-HIR rate (e.g. `M-HIR: 16.7%`)
- Active task goal (truncated; click to expand `TaskState` drawer)

### Session panel — conversation/episode view

Turns rendered as `<TurnCard>` components, newest at bottom.

```ts
interface TurnResult {
  id: string
  user_message: string
  tool_events: ToolEvent[]
  reply: string
  report: VerificationReport
  ts: number
}

const [turns, setTurns] = createStore<TurnResult[]>([])
const [pendingTokens, setPendingTokens] = createSignal('')
const [pendingTools, setPendingTools] = createStore<ToolEvent[]>([])

listen<string>('rk://token', e => setPendingTokens(t => t + e.payload))
listen<ToolEvent>('rk://tool_event', e => setPendingTools(t => [...t, e.payload]))
listen<TurnResult>('rk://turn_complete', e => {
  setTurns(t => [...t, e.payload])
  setPendingTokens('')
  setPendingTools([])
})
```

Fine-grained signal updates: only the token `<span>` re-renders during streaming,
not the full turn list.

#### `<TurnCard>`

```
┌─ Turn 3 ─────────────────────────────────── ✓ VERIFIED ─┐
│ User: Fix the null pointer in auth.rs                    │
│                                                          │
│ ▸ read_file("src/auth.rs")        ✓  0.1s               │
│ ▸ attribute_failure(validation_missing, validator)       │
│ ▸ edit_file("src/auth.rs")        ✓  0.1s  [diff ↗]    │
│ ▸ bash("cargo test auth")         ✓  2.3s               │
│                                                          │
│ Fixed: added empty-password check in validator.         │
│                                                          │
│ Checks: no_tool_errors ✓  clean_termination ✓           │
│ Limits: deterministic only; semantic not verified        │
└──────────────────────────────────────────────────────────┘
```

- Tool trace collapsed by default; expand per card
- `[diff ↗]` on `edit_file` opens editor tab in context panel
- Verification badge: ✓ green / ✗ red / spinner during pending
- Attribution badges on UNVERIFIED turns: `[constrain/policy]`
- Limits footnote always visible (never hidden)

#### `<TaskStateBanner>`

Persistent strip above conversation when a task is active:
- Goal text + criteria checklist (criteria checked off as judge fires)
- Status pill: Active / Done
- Click to expand full `TaskState` drawer

### Context panel

#### Terminal tab — xterm.js

```ts
const term = new Terminal({ allowTransparency: true })
term.loadAddon(new WebglAddon())
listen<string>('rk://bash_output', e => term.write(e.payload))
```

- Multi-tab: one `xterm.js` instance per named shell session
- Background process list (processes spawned by `bash_background`)
- Click a file path in output → opens that file in editor tab

#### Editor tab — CodeMirror 6 (diff-first)

```ts
const [diff, setDiff] = createSignal<FileDiff | null>(null)
listen<TurnResult>('rk://turn_complete', e => {
  const editEvent = e.payload.tool_events.findLast(t => t.name === 'edit_file')
  if (editEvent) setDiff(editEvent.diff)
})
```

- Renders the diff from the last `edit_file` / `write_file` by default
- Hunk-by-hunk accept/reject → `invoke('editor_accept_hunk', { ... })`
- Full file view toggle
- Syntax highlighting via CM6 language packages
- **Read-only by default** — direct edits recorded as `direct_edit` intervention
  via `invoke('record_intervention', { kind: 'direct_edit' })`

#### Git tab

- Staged / unstaged diff (same CM6 diff component, hunk granularity)
- Branch name + commits ahead/behind
- Auto-refreshes after turns containing git-touching bash calls
- "Commit" button → `invoke('session_command', { command: '/commit' })`

#### Memory tab

- Short-term stream: last N observations, newest first
- Long-term store: searchable by title/content
- **Recall highlight**: memories recalled in last turn highlighted in yellow
  (from `turn_complete.recalled_memories`)
- Skill browser: list with importance score, last used, grooming status
- Consolidation status bar: last run + next trigger count

#### Web tab

- Webview for `web_fetch` HTML results
- Card list for `web_search` results
- Read-only URL bar

### Composer

Full-width. Locked (grayed) while kernel is running.

```ts
const [locked, setLocked] = createSignal(false)
listen<void>('rk://turn_start', () => setLocked(true))
listen<TurnResult>('rk://turn_complete', () => setLocked(false))

const send = () => {
  if (!message().trim() || locked()) return
  invoke('session_send', { message: message(), attachments: attachments() })
  setMessage('')
  setAttachments([])
}
```

- Enter sends; Shift+Enter inserts newline
- `@path` → fuzzy file picker (`invoke('fs_list_workspace')`)
- `#name` → memory search (`invoke('session_memory_search', { q })`)
- `/` → command palette from `invoke('session_commands_list')`
- Input history: `↑`/`↓`; persisted to localStorage

#### Tool approval gate

Replaces textarea when `rk://approval_request` fires:

```
⚠ Agent wants to run:
bash("cargo test --workspace")

[Allow]  [Allow Always]  [Block]
```

`Block` → `invoke('approval_respond', { approved: false })` + records
`tool_block` intervention.

#### Plan mode confirmation

Renders when `rk://plan_exit` fires:

```
Agent has proposed a plan. Proceed with execution?

[Proceed]  [Reject]  [Annotate…]
```

"Annotate…" opens plan text in an editable overlay; annotation sent as
follow-up message.

### Harness dashboard

Accessible via `Cmd/Ctrl+Shift+H` or header icon. No equivalent in Claude Code
or hermes-agent.

All data from `invoke` calls; auto-refreshes on `rk://turn_complete`.

#### Verification stream

Newest-first list of turn outcomes:

```
Turn 12  ✓ VERIFIED   2 checks passed           14:03
Turn 11  ✗ UNVERIFIED tool_error @ feed/tools    14:02
```

Click to expand: full check results, attributions, limits, episode package link
(H3 only).

#### Evidence journal

Searchable, filterable table over `EvidenceEntry[]`. Filter by kind:
`turn` / `improvement` / `compaction` / `entropy`. Export via
`invoke('session_evidence_export')`.

#### Entropy panel

```ts
listen<EntropyAudit>('rk://entropy', e => setEntropy(h => [...h, e.payload]))
```

- Bar chart: entropy delta per turn (green = neutral, red = burden)
- Category breakdown: Residue / TestWeakening / StaleDocs / DependencyChurn /
  BoundaryViolation
- Finding list with file path links (click → editor tab)
- Cumulative score

#### M-HIR dashboard

```
M-HIR: 3 / 18 turns = 16.7%   Trend: ↓ improving

unverified_followup  2  ████████░░░
manual_verify        1  ████░░░░░░░
```

Trend sparkline across last 10 sessions. Each type clickable → jumps to turn.

#### Token budget

- Donut chart (plain SVG): used / limit
- Breakdown: input / output / cached tokens
- Compaction history: tier + timestamp
- Estimated cost (from provider pricing in Config)

### Settings panel

All config read/written via `invoke`. Keys stored in OS keychain.

- **AI / Model**: provider selector (13 providers), model picker with capability
  scores and pricing, embed model, test connection
- **API Keys**: masked fields; reveal via `invoke('secrets_get')`; stored in
  OS keychain — never localStorage
- **Permissions**: mode selector, security checker toggles, workspace path,
  web access toggle
- **MCP servers**: list from `invoke('mcp_servers_list')`; add/remove/test
- **Harness tuning**: max steps, recall K/window, idle threshold, groom threshold,
  compaction thresholds, harness level (H1/H2/H3)
- **Appearance**: 10 bundled themes, font size/family, vim mode toggle

## Seams

- **Voice input**: microphone in Composer when `RUSTYKEYS_VOICE=1`; Whisper
  via Tauri backend; transcription injected into input field (not auto-sent)
- **Multi-window**: separate `Session` per window; shared `Config`
- **Mobile / web**: the web gateway (PRD 06) exposes the same API; a
  browser-based frontend over it is a future adapter with no harness changes
- **OTel spans**: when aisdk adds OTel support, emit span events as Tauri events
  alongside the existing `rk://tool_event` stream
