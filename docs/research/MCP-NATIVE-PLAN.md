# MCP: replacing `rmcp` and its stack with first-party crates

Status: **proposed** (owner direction, 2026-10-08). Two independent tracks, joined by one rule.

## The rule that joins them

MCP crates depend on `rusty_request` / `rusty_serve` and **`rusty_tls`**, never on `rustls`, `reqwest` or `native-tls`. TLS work changes what is behind `rusty_tls`; the MCP crates inherit it with no edit. Until the MCP client reaches `rusty_request`, the interim `rusty-mcp-client` rides `rmcp` + `reqwest` on **rustls** (no OpenSSL).

## Track A: MCP (this track proceeds on today's `rusty_tls`)

| Step | Work | State |
|---|---|---|
| A0 | Amend workspace ADR-0002: MCP crates move from Tier A (`rmcp`, permanent) to Tier T (transitional) with this plan as the milestone list | done (Amendment 1, 2026-10-09) |
| A1 | Split the client out of `rusty-mcp` into `rusty-mcp-client`; client on rustls, not OpenSSL | done |
| A2 | `rusty_mcp_proto` (Tier S): JSON-RPC 2.0 + MCP types on `rusty_json`; `rmcp` as a **dev-only** wire-format oracle | in progress: type list complete in four slices (see `rusty_mcp_proto` README); scope in `MCP-PROTO-SCOPE.md` |
| A3 | Server on `rusty_serve` (stdio + stateless Streamable HTTP); accept on `rk-app` and `rusty-mcp-demo` | in progress: `rusty_serve` gained headers, `Limits` and `SharedHandler` (owner approved, additive); `rusty_mcp_server` has the core, stdio and stateless HTTP (tools, prompts, resources, completion, `subscriptions/listen`; verified against the `rmcp` client); **`rk-app` moved** (first consumer); tasks, multi-round-trip input (tools) and classic-HTTP sessions are in; **`rusty-mcp-demo` moved** (owner-approved swap, 43 acceptance tests over HTTP). server push streams with resumption and classic `resources/subscribe` are in. Remaining in A3: `notifications/tasks`, MRTR for prompts and resources, server-to-client requests, auth/limits/telemetry from `rusty-mcp` |
| A4 | Client on `rusty_request` + `rusty_tls`; needs an SSE reader | scoped in `MCP-CLIENT-SCOPE.md`; **built so far** as `rusty_mcp_client_native`: SSE parser, sans-IO session, blocking client over a child process and over Streamable HTTP on `rusty_request` (47 tests, incl. against `rmcp`'s own HTTP server); the async facade; `rusty-mcp-client` rebuilt on it (API changed in place, 2026-10-09) with `rk-mcp` moved. A gateway passthrough is next. Decisions taken by recommendation on 2026-10-08 (owner said go): new crate, async outside, tool-schema builder first |
| A5 | Move consumers one at a time: `rk-app`, `rk-mcp`, `rusty_homelab_mcp`, `rp-mcp`, ..., `agentgateway` last | `rk-app`, `rk-mcp` (via `rusty-mcp-client`), `rp-mcp` / `rp-server`, `adk-mcp`, `remind_me_remote`, `rusty_homelab_mcp` and `agentgateway` done (the gateway still uses `rmcp` through `rusty-mcp`'s telemetry and limits) |
| A6 | Drop `rmcp`, then `axum`/`clap`/`tracing-subscriber`/`jsonwebtoken` from the MCP crates | open; `rmcp` stays a dev-dependency oracle |

**Canonical implementation (owner, 2026-10-09):** #577. A parallel implementation of A2 and A3 in #586 was built without sight of #577 and has been dropped; #586 now carries only this plan and ADR-0002 Amendment 1. Its three review findings are recorded below as a checklist for any MCP server, #577 included.

Known gaps: tool-schema generation (`schemars` + `rmcp` macros; plan is a small schema builder first), SSE client in `rusty_request`, ES256 JWT verification (`rusty_oauth` has HS256/RS256), a native arg parser and logger to replace `clap`/`tracing-subscriber`, and whether `rusty_tokio` covers cancellation, child processes and signals.

## Track B: TLS (`rusty_tls`), run separately

Goal: the hand-rolled engine (`src/handrolled/`, about 10.5k lines, behind `handrolled-engine` + `--cfg rusty_tls_handrolled`) becomes a valid replacement for `rustls` behind the existing seam.

Blocking fact: `rusty_tls`'s own ADR-0002 records the gate as **permanently non-default**, on the argument that a wrong TLS stack fails silently in an attacker's favour. Making it default means superseding that ADR with an evidence bar. Suggested bar (owner to set): differential tests against `rustls` for handshake and certificate-path outcomes, known-answer tests, fuzzing, and an independent review of X.509 path building and signature verification. Open: replacing `ring` (AEAD) under the engine, and how far rusty_tls#25's test evidence already goes (not yet assessed).

Track B has its own owner/session; it does not block Track A.

### Checklist for any MCP stdio or HTTP server (from the review of #586)

1. Advertise only revisions that are fully implemented. A revision whose mandatory behaviour (for 2026-07-28: `server/discover` and `resultType` on results) is missing must not be negotiated, and a differential test must not hide the gap by stripping the member.
2. Check a request's cancellation *before* running a queued handler, so a cancelled mutating call never executes; test that the mutation does not happen.
3. When writing the response fails, shut down in bounded time even if the peer keeps its input open (the reader may be blocked on stdin); test a peer that closes its response pipe and holds its input pipe.

Session prompts for each track: `docs/research/prompts/mcp-track.md`, `docs/research/prompts/tls-track.md`.

## Decisions (owner, 2026-10-09)

1. ADR-0002 amendment: **yes** (done). 2. Tool schemas: **builder first**. (A builder was written in #586 and dropped with its crate; it is in that branch's history at `c2329ddd`, file `rusty_mcp_proto/src/schema.rs`, if #577 has no equivalent. #577's proto crate has no `schema` module.) 3. Blocking thread-per-connection servers on `rusty_serve`: **yes**, with additive changes now; an async driver on `rusty_tokio` behind a feature only if a forcing function appears (decided 2026-10-08). 4. Scope: **rmcp and its whole stack**. Still open: 5, the TLS evidence bar for making the native engine default (Track B).
