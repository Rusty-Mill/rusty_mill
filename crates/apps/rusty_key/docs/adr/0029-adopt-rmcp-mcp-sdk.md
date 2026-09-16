# ADR-0029: Adopt `rmcp` as the foundation of the `mcp` crate

- Status: Accepted
- Date: 2026-05-27
- Tags: mcp, dependencies, integration, licensing

## Context

PRD 07 originally specified hand-rolled JSON-RPC stdio/SSE transports for the
`mcp` crate. Round 2 (consolidated §ADOPT.1) found that `rmcp` — the official
`modelcontextprotocol/rust-sdk` — already provides this: it is **MIT-licensed,
tokio-native, and at v1.7.x** (verified). Re-implementing the transport layer
ourselves would be exactly the "custom component" the threat-model warns
against, and would diverge from the upstream protocol over time. The
opendocswork-mcp project was also assessed, but it is **GPL-3.0** and therefore
unusable as a dependency in an MIT/permissive tree.

## Decision

Adopt **`rmcp` (v1.7.x)** as the foundation of the `mcp` crate. PRD 07's
transports become **thin adapters over `rmcp`**, not bespoke JSON-RPC. Our
value-add — server namespacing, `McpPolicy`, the `ApprovalGate` integration,
and auth/TLS pinning — sits *above* `rmcp`, unchanged in intent. opendocswork-mcp
(GPL-3.0) is **reference-only**: its layout may inform ours, but it is never
vendored or linked. To enforce this, add a **`cargo deny` license gate** to CI
that rejects copyleft licenses in the dependency tree. Detail: `docs/prd/07-mcp.md`.

## Consequences

- The `mcp` crate gains a first-party upstream dependency; protocol changes are
  tracked by bumping `rmcp` rather than re-deriving the wire format.
- The `cargo deny` license gate is now load-bearing CI, protecting the MIT
  posture; a GPL transitive dependency will fail the build.
- Our trust/policy seams (`McpPolicy`, `ApprovalGate`, auth/TLS) are unaffected
  by the swap — they were always specified to sit above the transport.
- opendocswork-mcp stays a documented reference in `docs/prd/07-mcp.md`; treating
  it as reference-only is a deliberate licensing constraint, not an oversight.
