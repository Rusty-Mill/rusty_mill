# ADR-0016: `before_tool` becomes `async fn`

- Status: Accepted
- Date: 2026-05-27
- Tags: constrain, async, policy

## Context

PRD 02 defines `Policy::before_tool()` as synchronous (ADR-0007) on the grounds
that policy decisions must be fast and must not make network calls. The same PRD
then describes the `ApprovalGate`, which sends an approval request over an `mpsc`
channel and awaits a human response — an operation that cannot be synchronous.
This is a spec-internal contradiction (consolidated plan §A.2). Remote ACL
lookup is also a stated seam in PRD 02 that the sync signature would block.

## Decision

Make `Policy::before_tool` an `async fn`. The `ApprovalGate` is the concrete need
that forces it; remote ACL lookup is the natural extension of the same pattern.
The change propagates to `ToolRegistry::dispatch` and the kernel, which must
`await` the policy check before invoking a tool. See `docs/prd/02-constrain.md`
and `docs/ARCHITECTURE.md` for the propagated signatures.

## Consequences

- `before_tool`, `ToolRegistry::dispatch`, and the kernel dispatch path all
  become `async`; this is a breaking change to those signatures.
- Sequence this change before MCP / gateway work, since both build on the tool
  dispatch path (see `BACKLOG.md`).
- Policy implementations that do pure in-process logic incur no real cost; the
  async boundary is what enables the human-in-the-loop and remote-ACL seams.
