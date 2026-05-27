# PRD 02 — Constrain

## Responsibility

The constrain layer vets every tool call **before** the kernel dispatches it.
It is the harness's enforcement point: the place where permissions, boundaries,
and safety invariants are checked before any side effect occurs. The check itself
is `async` (ADR-0016) so it can await a human approval or a remote lookup, but the
common in-process path resolves without awaiting anything.

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
#[async_trait]
pub trait Policy: Send + Sync {
    async fn before_tool(&self, name: &str, args: &serde_json::Value)
        -> Result<(), PolicyError>;
}
```

`before_tool` is **`async fn`** (ADR-0016). It was originally synchronous on the
grounds that policy decisions must be fast and must not make network calls — but
the `ApprovalGate` (below) awaits a human response over a channel, and remote ACL
lookup is a stated seam. Both need an async boundary, so the signature is now
async; this also propagates to `ToolDispatch::dispatch` and the kernel dispatch
path (PRD 01), which `await` the policy check. **This was previously flagged as a
breaking change to be decided later; it is now decided.** Pure in-process policies
(the common case) incur no real cost — they simply don't `.await` anything inside.

`PolicyError` is a `thiserror` **enum**, not a `{ reason: String }` struct
(ADR-0023). Structured variants let failure attribution (PRD 05) and the
`security.jsonl` `checker` field derive *structurally* rather than by parsing free
prose:

```rust
#[derive(thiserror::Error, Debug)]
pub enum PolicyError {
    #[error("path {0} escapes workspace root")]
    OutsideWorkspace(PathBuf),
    #[error("mode {mode} forbids tool {tool}")]
    ModeForbidden { mode: &'static str, tool: String },
    #[error("blocked by {checker}: {pattern}")]
    SecurityCheck { checker: &'static str, pattern: String },
    #[error("web access disabled; set RUSTYKEYS_ALLOW_WEB=1")]
    WebDisabled,
    #[error("egress blocked: {host} is on the {list} list")]
    EgressBlocked { host: String, list: &'static str },
    #[error("blocked by approval gate")]
    ApprovalDenied,
}
```

Each variant maps onto an attribution `(category, layer)` pair in PRD 05 and onto
a structured record in `security.jsonl`. The full taxonomy and `#[from]`
composition live in the forthcoming `docs/dev/error-handling.md` (ADR-0023); the
enum's serde encoding (snake_case) is owned by
[`data-model.md` §7](../architecture/data-model.md#7-serde-wire-conventions-adr-0025).
The variant *name* (e.g. `SecurityCheck`'s `checker`) is what
[`security.jsonl`](../architecture/data-model.md#43-securityjsonl--blocked-calls-prd-02)
records — not the rendered `Display` string.

### `WorkspacePolicy` (default implementation)

The default policy enforces a filesystem allowlist: any tool that reads or writes
files must operate within the configured workspace root.

```rust
pub struct WorkspacePolicy {
    workspace_root: PathBuf,
    web_allowed: bool,
    web_egress: WebEgressGuard,   // SSRF / allow-deny for web_fetch + web_search
    bash_guard: BashGuard,        // security checkers, constructed at Session::new()
    mode: PermissionMode,
}

#[async_trait]
impl Policy for WorkspacePolicy {
    async fn before_tool(&self, name: &str, args: &serde_json::Value) -> Result<(), PolicyError> {
        // Mode-level gate first
        self.mode.check(name)?;
        // Filesystem boundary
        if is_filesystem_tool(name) {
            let path = args["path"].as_str().unwrap_or("");
            self.check_within_workspace(path)?;   // → PolicyError::OutsideWorkspace
        }
        // Web access gate
        if is_web_tool(name) {
            if !self.web_allowed {
                return Err(PolicyError::WebDisabled);
            }
            // Egress guard: block SSRF targets + apply allow/deny lists
            self.web_egress.check(args)?;          // → PolicyError::EgressBlocked
        }
        // Security checkers (bash only, today — see web egress note)
        if name == "bash" {
            self.bash_guard.check(args)?;          // → PolicyError::SecurityCheck { checker, pattern }
        }
        Ok(())
    }
}
```

Rust's `Path::canonicalize()` and prefix checking make the boundary check
correct by construction — no string-prefix tricks that could be fooled by `../`.
The `async fn` boundary is what lets the `ApprovalGate` (below) await a human
decision from inside this method without blocking the runtime.

### Web-egress policy hook

`web_fetch` / `web_search` take an arbitrary URL. Gating them on the boolean
`RUSTYKEYS_ALLOW_WEB` alone is not enough: an agent with web enabled could reach
cloud metadata or internal services (SSRF). A `WebEgressGuard` runs inside
`before_tool` for web tools and is the analogue of `NetworkExfilCheck` — which
**today only inspects `bash` args** and does not cover the `web_*` tools at all.

```rust
pub struct WebEgressGuard {
    allowlist: Vec<String>,   // RUSTYKEYS_WEB_ALLOWLIST (empty ⇒ allow all non-blocked)
    denylist:  Vec<String>,   // RUSTYKEYS_WEB_DENYLIST (extends the built-in SSRF set)
}

impl WebEgressGuard {
    pub fn check(&self, args: &serde_json::Value) -> Result<(), PolicyError>;
}
```

The guard resolves the target host and **always blocks** (built-in, not
overridable): loopback (`127.0.0.0/8`, `::1`), RFC1918 private ranges
(`10/8`, `172.16/12`, `192.168/16`), link-local (`169.254.0.0/16`, `fe80::/10`),
and the cloud-metadata address `169.254.169.254`. On top of that, an empty
`RUSTYKEYS_WEB_ALLOWLIST` means "allow any non-blocked host"; a non-empty
allowlist is restrictive (only those hosts); `RUSTYKEYS_WEB_DENYLIST` extends the
built-in block set. A blocked target returns `PolicyError::EgressBlocked { host,
list }` (rendered to the model as a `BLOCKED` string like any other policy
rejection). Redirect-following and response-size caps are owned by the `web_*`
tools in [PRD 03](./03-feed.md); the *destination* decision lives here.
The env vars are defined in
[`configuration.md` › Tools](../reference/configuration.md#tools); the full
SSRF/egress rule is owned by the forthcoming `docs/architecture/threat-model.md`.

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

A registry of **synchronous** checks run inside `WorkspacePolicy::before_tool()`
for `bash` calls. They stay sync — pure pattern matching with no I/O — and are
invoked from the now-`async` `before_tool` without awaiting. Each implements
`SecurityCheck`:

```rust
pub trait SecurityCheck: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, args: &serde_json::Value) -> Result<(), PolicyError>;
}
```

A failed check returns `PolicyError::SecurityCheck { checker, pattern }`, so the
checker name and matched pattern reach the security log structurally (below).

Built-in checkers:

| Checker | What it blocks |
|---|---|
| `CommandInjectionCheck` | `;`, `&&`, `\|\|` followed by network/delete; pipe-to-shell (`curl … \| sh`) |
| `PrivilegeEscalationCheck` | `sudo`, `su -`, `chmod 777`, `chown root` |
| `PathTraversalCheck` | `../../`, absolute paths outside workspace in bash args |
| `NetworkExfilCheck` | `curl`/`wget`/`nc` piped to external IPs **inside `bash`** |
| `DestructiveCommandCheck` | `rm -rf /`, `git reset --hard`, SQL `DROP TABLE` |

> **`NetworkExfilCheck` covers `bash` only.** It does **not** inspect the `web_*`
> tools — those go through the separate `WebEgressGuard` (above). The two are
> complementary: `NetworkExfilCheck` catches shell-level exfil, `WebEgressGuard`
> catches SSRF/egress through the first-class web tools.

Each blocked call writes a `SecurityEvent` to `.rustykeys/security.jsonl`
(append-only, separate from `interventions.jsonl`). The `checker` field is the
structured `PolicyError::SecurityCheck` variant name (ADR-0023), not free prose,
and `args` is **redacted before it is written** (secret redaction is on by
default — see below):

```json
{"v":1,"ts":1234567890.5,"session_id":"s_abc","tool":"bash",
 "checker":"CommandInjectionCheck","pattern":"curl … | sh",
 "args":{"command":"<redacted>"}}
```

The on-disk schema for this record is owned by
[`data-model.md` §4.3](../architecture/data-model.md#43-securityjsonl--blocked-calls-prd-02).

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

### Secret redaction (required default)

Tool `args` and results can carry secrets — MCP/SSE auth tokens, `web_*` API
keys, anything the model passes as an argument. **Redaction is on by default**
(ADR-0026): it is no longer a future `RedactPolicy` seam but a required pass
applied at the observe/emit boundary *before* any `ToolEvent` is written to
`evidence.jsonl` / `security.jsonl`, embedded in an episode package, or emitted
over `/evidence` or `rk://tool_event`. (See the redacted `args` in the
`security.jsonl` example above.)

The default scrub replaces values for argument keys matching `*token*`, `*key*`,
`*secret*`, `auth*`, `password*` with `"<redacted>"`, and scrubs high-entropy
value patterns (long base64/hex) in string fields. It scrubs **values, not
structure**, so attribution/verification traces that depend on which tool ran
with which shape of args are preserved. Controlled by `RUSTYKEYS_REDACT`
(default `1`; disabling is strongly discouraged). The exact deny-list and value
patterns are owned by the forthcoming `docs/architecture/threat-model.md`
(ADR-0026); the env var is in
[`configuration.md` › Permissions & safety](../reference/configuration.md#permissions--safety).

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
calls skip the gate. `Block` causes `before_tool` to return
`PolicyError::ApprovalDenied` and records a `tool_block` intervention in the
`InterventionLogger`.

Awaiting the human response over the `mpsc` channel is exactly why `before_tool`
is `async fn` (ADR-0016). This was previously written as a *planned breaking
change* deferred to a future Seam; **it is now decided and in effect** — the
trait, the `WorkspacePolicy` impl, and `ToolDispatch::dispatch` are all async
(see PRD 01's dispatch path). No outstanding "becomes async" caveat remains.

### Composition

Multiple policies can be composed via a `PolicyChain`:

```rust
pub struct PolicyChain(Vec<Box<dyn Policy>>);

#[async_trait]
impl Policy for PolicyChain {
    async fn before_tool(&self, name: &str, args: &serde_json::Value) -> Result<(), PolicyError> {
        for policy in &self.0 {
            policy.before_tool(name, args).await?;
        }
        Ok(())
    }
}
```

This allows workspace policy, rate-limit policy, and capability-gate policy
to be stacked without each knowing about the others.

## Integration with the tool registry

`ToolDispatch::dispatch()` (implemented by `feed::ToolRegistry`) **`await`s**
`policy.before_tool()` before invoking the tool function — the check is now async
(ADR-0016). A `PolicyError` is rendered to a `BLOCKED by policy: …` string via a
structured `ToolOutcome` (ADR-0022), recorded by the `Tracer` with
`status = blocked`, and returned to the aisdk loop as the tool result. The
`Tracer` reads `ToolOutcome.status` directly — it does **not** re-parse the
`BLOCKED`/`ERROR` prefix out of the string.

```rust
async fn dispatch(&self, name: &str, args: Value, policy: &dyn Policy,
                  tracer: &mut Tracer) -> String {
    if let Err(e) = policy.before_tool(name, &args).await {
        // e is a structured PolicyError variant; Display renders the reason
        let outcome = ToolOutcome::Blocked(e.to_string());
        tracer.record_tool(name, &args, &outcome);   // status derives from the variant
        return outcome.to_model_string();             // "BLOCKED by policy: …"
    }
    match self.tools.get(name) {
        Some(tool) => {
            let outcome = tool.call(args).await;       // ToolFn::call -> ToolOutcome
            tracer.record_tool(name, &args, &outcome);
            outcome.to_model_string()
        }
        None => ToolOutcome::Error(format!("unknown tool '{name}'")).to_model_string(),
    }
}
```

The `PolicyError` *variant* (not the rendered string) is what feeds the
structured `security.jsonl` `checker` field and the PRD 05 attribution
`(category, layer)` mapping. The `ToolOutcome` type and its formatter/parser are
owned by [`data-model.md` §7](../architecture/data-model.md#7-serde-wire-conventions-adr-0025)
and the forthcoming `docs/dev/error-handling.md` (ADR-0022/0023). `dispatch` is
part of the `ToolDispatch` trait the kernel receives as `&dyn ToolDispatch`
(PRD 01), which is what keeps `kernel → feed` out of the dependency DAG.

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

## Decided (no longer seams)

- **Async policy**: `before_tool` is now `async fn` (ADR-0016), driven by the
  `ApprovalGate`. Remote ACL lookup is the natural extension of the same async
  boundary.
- **Argument redaction**: secret redaction is now a **required default**
  (ADR-0026), not an optional `RedactPolicy`. See *Secret redaction* above.
- **Web egress**: SSRF/egress control for `web_*` is now a first-class
  `WebEgressGuard` hook (above), not a deferred idea.

## Seams

- **Remote ACL policy**: an async `AclPolicy` that calls out to an external
  authorization service — enabled by the now-async `before_tool`.
- **Rate limiting**: a `RateLimitPolicy` tracking call counts per tool per
  sliding window.
- **Audit log rotation**: `security.jsonl` grows unbounded; a rotation/retention
  policy is a future seam (ADR-0015).
- **MCP policy**: an `McpPolicy` allowlists/blocklists MCP server names and tool
  names — implemented in PRD 07.
