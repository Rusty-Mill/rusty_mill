# PRD 02 — Constrain

## Responsibility

The constrain layer vets every tool call **before** the kernel dispatches it.
It is the harness's enforcement point: the place where permissions, boundaries,
and safety invariants are checked synchronously, before any side effect occurs.

If the policy rejects a call, the kernel sees a `BLOCKED` result string and can
recover. The process never panics on a policy violation.

## Why this layer exists

Once an agent has tools, a wrong inference becomes a destructive *action* —
deleted files, bad commits, exfiltrated data. The constrain layer narrows the
scope of what the agent can do. Per Ashby's Law, the harness must have at least
as much variety as the system it governs; the constrain layer is where that
variety-reduction happens concretely.

## Design

### `Policy` trait

```rust
pub trait Policy: Send + Sync {
    fn before_tool(&self, name: &str, args: &serde_json::Value)
        -> Result<(), PolicyError>;
}

pub struct PolicyError {
    pub reason: String,
}
```

`before_tool` is synchronous — policy decisions must be fast and must not
themselves make network calls. Async policy (e.g. remote ACL lookup) is a future
seam; today policy is pure in-process logic.

### `WorkspacePolicy` (default implementation)

The default policy enforces a filesystem allowlist: any tool that reads or writes
files must operate within the configured workspace root.

```rust
pub struct WorkspacePolicy {
    workspace_root: PathBuf,
    web_allowed: bool,
    mode: PermissionMode,
}

impl Policy for WorkspacePolicy {
    fn before_tool(&self, name: &str, args: &serde_json::Value) -> Result<(), PolicyError> {
        // Mode-level gate first
        self.mode.check(name)?;
        // Filesystem boundary
        if is_filesystem_tool(name) {
            let path = args["path"].as_str().unwrap_or("");
            self.check_within_workspace(path)?;
        }
        // Web access gate
        if is_web_tool(name) && !self.web_allowed {
            return Err(PolicyError { reason: "web access disabled; set RUSTYKEYS_ALLOW_WEB=1".into() });
        }
        // Security checkers (bash only)
        if name == "bash" {
            BashGuard::check(args)?;
        }
        Ok(())
    }
}
```

Rust's `Path::canonicalize()` and prefix checking make the boundary check
correct by construction — no string-prefix tricks that could be fooled by `../`.

### `PermissionMode`

Named permission modes control which tool classes are allowed. Set via
`RUSTYKEYS_PERMISSION_MODE` or the `/permissions` CLI command.

```rust
pub enum PermissionMode {
    /// Default: workspace boundary enforced; bash allowed with security checks.
    Default,
    /// Plan: no filesystem writes or bash; read-only + propose only.
    Plan,
    /// AcceptEdits: writes allowed without approval prompt; bash still checked.
    AcceptEdits,
    /// ReadOnly: no writes, no bash; read_file, list_directory, glob, grep, web_fetch only.
    ReadOnly,
    /// Restricted: only tools in the explicit allowlist.
    Restricted { allowed_tools: Vec<String> },
    /// Bypass: all policy checks disabled. Requires RUSTYKEYS_ALLOW_BYPASS=1.
    Bypass,
}

impl PermissionMode {
    pub fn check(&self, tool_name: &str) -> Result<(), PolicyError> { ... }
}
```

| Mode | Writes | Bash | Web | Notes |
|---|---|---|---|---|
| Default | ✓ (approval) | ✓ (checked) | opt-in | Standard operation |
| Plan | ✗ | ✗ | ✓ | Agent proposes; human approves before switch |
| AcceptEdits | ✓ (no prompt) | ✓ (checked) | opt-in | CI / non-interactive use |
| ReadOnly | ✗ | ✗ | ✓ | Safe exploration |
| Restricted | allowlist | allowlist | allowlist | Custom per-session |
| Bypass | ✓ | ✓ | ✓ | Explicit opt-in only |

### Security checkers

A registry of synchronous checks run inside `WorkspacePolicy::before_tool()`
for `bash` calls. Each implements `SecurityCheck`:

```rust
pub trait SecurityCheck: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, args: &serde_json::Value) -> Result<(), PolicyError>;
}
```

Built-in checkers:

| Checker | What it blocks |
|---|---|
| `CommandInjectionCheck` | `;`, `&&`, `\|\|` followed by network/delete; pipe-to-shell (`curl … \| sh`) |
| `PrivilegeEscalationCheck` | `sudo`, `su -`, `chmod 777`, `chown root` |
| `PathTraversalCheck` | `../../`, absolute paths outside workspace in bash args |
| `NetworkExfilCheck` | `curl`/`wget`/`nc` piped to external IPs inside bash |
| `DestructiveCommandCheck` | `rm -rf /`, `git reset --hard`, SQL `DROP TABLE` |

Each blocked call writes a `SecurityEvent` to `.rustykeys/security.jsonl`
(append-only, separate from `interventions.jsonl`):

```json
{"ts": 1234567890.5, "tool": "bash", "checker": "CommandInjectionCheck",
 "pattern": "curl … | sh", "args": {"command": "…"}}
```

### `BashGuard`

Aggregates all `SecurityCheck` implementations into a single synchronous call:

```rust
pub struct BashGuard {
    checkers: Vec<Box<dyn SecurityCheck>>,
    security_log: PathBuf,
}

impl BashGuard {
    pub fn check(&self, args: &serde_json::Value) -> Result<(), PolicyError>;
}
```

`BashGuard` is constructed once at `Session::new()` and held by `WorkspacePolicy`.

### `ApprovalGate`

For tool calls that pass all automated checks but still warrant human attention
(first bash use, writing to a new path, first MCP tool use), the policy sends
an approval request through a channel pair:

```rust
pub struct ApprovalGate {
    pub triggers: Vec<ApprovalTrigger>,
    pub tx: mpsc::Sender<ApprovalRequest>,
    pub rx: mpsc::Receiver<ApprovalResponse>,
}

pub enum ApprovalTrigger {
    BashFirstUse,
    WriteOutsideGitRepo,
    NewFilePath,
    McpToolFirstUse { server: String },
}

pub struct ApprovalRequest {
    pub tool: String,
    pub args: serde_json::Value,
    pub trigger: ApprovalTrigger,
}

pub enum ApprovalResponse {
    Allow,
    AllowAlways,  // adds tool to session auto-approve list
    Block,
}
```

The CLI / desktop adapter listens on the channel and renders the approval prompt.
`AllowAlways` adds the tool name to an in-session `HashSet<String>`; subsequent
calls skip the gate. `Block` causes `before_tool` to return `PolicyError` and
records a `tool_block` intervention in the `InterventionLogger`.

The `ApprovalGate` requires `before_tool` to become `async fn` — this is the
planned breaking change described in the original Seams. It is accepted here
as the use case is now concrete.

### Composition

Multiple policies can be composed via a `PolicyChain`:

```rust
pub struct PolicyChain(Vec<Box<dyn Policy>>);

impl Policy for PolicyChain {
    fn before_tool(&self, name: &str, args: &serde_json::Value) -> Result<(), PolicyError> {
        for policy in &self.0 {
            policy.before_tool(name, args)?;
        }
        Ok(())
    }
}
```

This allows workspace policy, rate-limit policy, and capability-gate policy
to be stacked without each knowing about the others.

## Integration with the tool registry

`ToolRegistry::dispatch()` calls `policy.before_tool()` before invoking the
tool function. A `PolicyError` is converted to a `BLOCKED by policy: {reason}`
string, recorded by the `Tracer` with `status = blocked`, and returned to the
aisdk loop as the tool result.

```rust
pub async fn dispatch(&self, name: &str, args: Value, policy: &dyn Policy,
                      tracer: &mut Tracer) -> String {
    if let Err(e) = policy.before_tool(name, &args) {
        let result = format!("BLOCKED by policy: {}", e.reason);
        tracer.record_tool(name, &args, &result);
        return result;
    }
    match self.tools.get(name) {
        Some(tool) => {
            let result = tool.call(args).await;
            tracer.record_tool(name, &args, &result);
            result
        }
        None => format!("ERROR: unknown tool '{name}'"),
    }
}
```

## Rust advantages here

- **Ownership**: `WorkspacePolicy` holds the resolved `PathBuf`; no string
  aliasing bugs possible.
- **`Send + Sync`**: the `Policy` trait bound ensures policies are safe to use
  from async contexts without a `Mutex`.
- **`Path::canonicalize()`**: resolves symlinks and `..` components correctly,
  making the workspace boundary airtight at the type level.
- **No runtime cost**: `PolicyChain` iterates a `Vec` of trait objects — the
  hot path is a handful of pointer dereferences, not Python interpreter overhead.
- **Enum-based modes**: `PermissionMode` is exhaustively checked at compile time;
  adding a new mode requires handling it everywhere.

## Seams

- **Async policy**: `before_tool` is now `async fn` via `ApprovalGate`. Remote
  ACL lookup is a natural extension of the same pattern.
- **Rate limiting**: a `RateLimitPolicy` tracking call counts per tool per
  sliding window.
- **Argument redaction**: a `RedactPolicy` that scrubs secrets from `args`
  before the Tracer records them.
- **Audit log rotation**: `security.jsonl` grows unbounded; a rotation/retention
  policy is a future seam.
- **MCP policy**: an `McpPolicy` allowlists/blocklists MCP server names and tool
  names — implemented in PRD 07.
