//! Hister's MCP JSON-RPC tool surface — a Rust port of `server/mcp.go`,
//! built on [`rusty_mcp`](../../../rusty_mcp)'s existing MCP server
//! framework (spec `2026-07-28`, on `rmcp`) rather than a fresh JSON-RPC
//! implementation — the sovereignty audit in
//! `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md` confirmed
//! `rusty_mcp` already has a mature `#[tool_router]`/`#[tool]`
//! tool-registration API this crate's three tools can register against.
//!
//! **Bootstrap stage — no implementation yet.** This is the second v1
//! deliverable alongside `rusty-hister-server`. See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §2 for the
//! **highest-priority verbatim text in this whole port**: the `search`,
//! `get_preview`, and `get_history` tool descriptions, the
//! trust-boundary/`untrusted_content` envelope shape, the
//! `mcpUntrustedContentInstruction` constant, and `mcpNormalizeUntrusted`'s
//! Unicode-category-based control-character stripping — all of it must ship
//! byte-for-byte equivalent, not paraphrased, since this is Hister's
//! prompt-injection defense surface.
