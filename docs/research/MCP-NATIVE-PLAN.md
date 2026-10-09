# MCP: replacing `rmcp` and its stack with first-party crates

Status: **proposed** (owner direction, 2026-10-08). Two independent tracks, joined by one rule.

## The rule that joins them

MCP crates depend on `rusty_request` / `rusty_serve` and **`rusty_tls`**, never on `rustls`, `reqwest` or `native-tls`. TLS work changes what is behind `rusty_tls`; the MCP crates inherit it with no edit. Until the MCP client reaches `rusty_request`, the interim `rusty-mcp-client` rides `rmcp` + `reqwest` on **rustls** (no OpenSSL).

## Track A: MCP (this track proceeds on today's `rusty_tls`)

| Step | Work | State |
|---|---|---|
| A0 | Amend workspace ADR-0002: MCP crates move from Tier A (`rmcp`, permanent) to Tier T (transitional) with this plan as the milestone list | done (Amendment 1, 2026-10-09) |
| A1 | Split the client out of `rusty-mcp` into `rusty-mcp-client`; client on rustls, not OpenSSL | done |
| A2 | `rusty_mcp_proto` (Tier S): JSON-RPC 2.0 + MCP types on `rusty_json`; `rmcp` as a **dev-only** wire-format oracle | slices 1 and 2 done (P0 + P1: JSON-RPC, initialize, tools, resources, prompts, completion, schema builder); P2 (`resultType`, tasks, elicitation, subscriptions) when `rusty-mcp` itself moves |
| A3 | Server on `rusty_serve` (stdio + stateless Streamable HTTP); accept on `rk-app` and `rusty-mcp-demo` | stdio done (`rusty_mcp_server`, `rmcp` client acceptance passes); `rk-app` moved; resources/prompts/completion handlers done; stateless Streamable HTTP done (feature `http`, `rmcp` HTTP client acceptance passes). Next: A4 client on `rusty_request`/`rusty_tls` (needs an SSE reader), then consumers (`rusty_homelab_mcp` first), `rusty-mcp-demo` last as it is macro-heavy |
| A4 | Client on `rusty_request` + `rusty_tls`; needs an SSE reader | open |
| A5 | Move consumers one at a time: `rk-app`, `rk-mcp`, `rusty_homelab_mcp`, `rp-mcp`, ..., `agentgateway` last | open |
| A6 | Drop `rmcp`, then `axum`/`clap`/`tracing-subscriber`/`jsonwebtoken` from the MCP crates | open |

Known gaps: tool-schema generation (`schemars` + `rmcp` macros; plan is a small schema builder first), SSE client in `rusty_request`, ES256 JWT verification (`rusty_oauth` has HS256/RS256), a native arg parser and logger to replace `clap`/`tracing-subscriber`, and whether `rusty_tokio` covers cancellation, child processes and signals.

## Track B: TLS (`rusty_tls`), run separately

Goal: the hand-rolled engine (`src/handrolled/`, about 10.5k lines, behind `handrolled-engine` + `--cfg rusty_tls_handrolled`) becomes a valid replacement for `rustls` behind the existing seam.

Blocking fact: `rusty_tls`'s own ADR-0002 records the gate as **permanently non-default**, on the argument that a wrong TLS stack fails silently in an attacker's favour. Making it default means superseding that ADR with an evidence bar. Suggested bar (owner to set): differential tests against `rustls` for handshake and certificate-path outcomes, known-answer tests, fuzzing, and an independent review of X.509 path building and signature verification. Open: replacing `ring` (AEAD) under the engine, and how far rusty_tls#25's test evidence already goes (not yet assessed).

Track B has its own owner/session; it does not block Track A.

Session prompts for each track: `docs/research/prompts/mcp-track.md`, `docs/research/prompts/tls-track.md`.

## Decisions (owner, 2026-10-09)

1. ADR-0002 amendment: **yes** (done). 2. Tool schemas: **builder first** (done: `rusty_mcp_proto::Schema`). 3. Blocking thread-per-connection servers on `rusty_serve`: **yes**. 4. Scope: **rmcp and its whole stack**. Still open: 5, the TLS evidence bar for making the native engine default (Track B).
