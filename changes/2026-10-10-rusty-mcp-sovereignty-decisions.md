---
category: Changed
changelog: **`rusty-mcp` sovereignty plan: owner decisions recorded.** Cores plus adapters, OTLP/HTTP+JSON, `tracing` kept for now, `tower`/`http`/`tokio` accepted as named exceptions, JWT parity first. Two items added: more JWT algorithms (PS256, ES384, EdDSA) and an in-house `tracing` facade.
---
## 2026-10-10 - `rusty-mcp` sovereignty plan: decisions D1 to D5

- **Decided:** D1 sans-IO cores with a `tower` adapter and a `rusty_serve` adapter; D2 OTLP over HTTP with JSON; D3 keep `tracing`; D4 `tower-layer`, `tower-service`, `http` and `tokio` stay as named exceptions until the gateway leaves axum; D5 RS256 and ES256 parity first.
- **Added:** item 11 (PS256, ES384, EdDSA) and item 12 (in-house `tracing` facade, workspace-wide, needs its own plan) in `docs/research/RUSTY-MCP-SOVEREIGNTY.md`.
