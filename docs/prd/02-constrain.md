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
}

impl Policy for WorkspacePolicy {
    fn before_tool(&self, name: &str, args: &serde_json::Value) -> Result<(), PolicyError> {
        if is_filesystem_tool(name) {
            let path = args["path"].as_str().unwrap_or("");
            self.check_within_workspace(path)?;
        }
        Ok(())
    }
}
```

Rust's `Path::canonicalize()` and prefix checking make the boundary check
correct by construction — no string-prefix tricks that could be fooled by `../`.

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

## Seams

- **Interactive approval**: `before_tool` could `await` a channel send to the
  CLI for a y/n prompt before high-risk tool calls. Requires the trait to become
  `async fn`, which is a breaking change — deferred until there's a use case.
- **Rate limiting**: a `RateLimitPolicy` tracking call counts per tool per
  sliding window.
- **Argument redaction**: a `RedactPolicy` that scrubs secrets from `args`
  before the Tracer records them.
- **Capability gating**: deny specific tools entirely for a given session
  (e.g. no filesystem writes in read-only mode).
- **Audit logging**: the `Tracer` already records blocked calls; a dedicated
  policy audit log is a future seam here.
