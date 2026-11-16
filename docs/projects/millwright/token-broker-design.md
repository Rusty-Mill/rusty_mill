# Design note: GitHub App token broker (G1 follow-up, option b)

Status: **proposal; nothing built.** Nano approved drafting this and a minimal build on
2026-11-16 ("yes"). The build waits on the ask-first items at the end. Written 2026-11-16.

## Problem
G1 gives each agent role its own GitHub identity (Apps, O2). Cloud sessions cannot hold App
private keys: environment secrets are shared by every session in a project, and a thread
cannot pick its environment. A key there is a key every session can read, including a
builder reading the reviewer's.

## Proposal
A small service, **`rusty_token_broker`**, holds each App's ID, installation ID and private
key. A session proves who it is, asks for a role, and gets a 1-hour installation token
narrowed to that role. No session holds an App key.

### Where it runs and how sessions reach it
- **A separate process from Mill Board.** Fault isolation is the forcing function: the board
  has a web UI and keeps agent keys in clear; the broker holds signing keys and nothing else.
- **Host: BaileyAi** (recommended; no recorded decision covers it, see ask-first 1).
- **Reached as an MCP connector**, like Mill Board and remind-me. Sessions already reach
  connectors, so no environment network allowlist change and no shared secret. One tool:
  `token_mint {role, repo}` returns `{token, expires_at, permissions, repos}`.
  Tradeoff: the token passes through the model's context. It is 1 hour and one repo.
  A later `gh_proxy` tool (broker makes the call, token never leaves) removes this.

### How a session proves which role it may ask for
Three layers, weakest first; the honest limit is stated below.
1. **Connector sign-in** says what is connecting (cloud Claude). Not a role.
2. **Board agent identity.** The session calls `agent_register` ([agent-register-design.md])
   and passes `agent_id` + `agent_key` to the broker. The broker asks Mill Board to verify
   them and return the agent's role. The role is the board's, not the session's claim.
   A self-narrowed role can never exceed the connector's, so with the connector at
   `builder` a session cannot become `reviewer` (no `review` capability). The reviewer
   lane gets `reviewer` only through an admin grant (`settings_registered_agent_set`),
   which Nano controls. This reuses the registry; no second one.
3. **Task binding (v2).** Reviewer tokens only for a PR whose card is in Review and not
   claimed by the requester; builder tokens only for a claimed Work-column card. A leaked
   key without a live claim gets nothing.

Limit: a prompt-injected session can leak its own `agent_key`; blast radius is one agent
id, one role, one repo at a time, revocable alone. Same accepted tradeoff as `agent_register`.
Not isolation between hostile sessions.

### Role to App mapping and scope (config, not code)
`role -> {app_id, installation_id, key_file, permissions, repos}`. Requests may only
narrow. Starting point from g1-identity.md answer 2: builder contents write, pull requests
write, checks read, metadata read; reviewer and qa the same minus contents write. No
administration, no workflow write. Repos limited to the allowlist; the token names the
single requested repo (`repositories: [name]`).
Drift: Nano's eight Apps (2026-11-15) are per agent, not per role. The mapping is
many-to-one so either works; Nano picks which App serves which role.

### Key storage
PEM files, mode 0600, owned by a dedicated service user, delivered by systemd
`LoadCredential=`. Never in a repo, env var, log or chat. Loaded once; `Debug` redacted.
Non-secret IDs in a config file. Rotation: GitHub allows two keys per App; add, restart,
delete old. Nano installs keys himself.

### Expiry
GitHub fixes installation tokens at 1 hour. The broker mints per request (limit 5000/h per
installation is ample), no cache in v0, and returns `expires_at`. Per-agent rate limit.

### Audit
Append-only JSONL (0600) plus `rusty_log`: time, agent id, role asked, role resolved, repo,
permissions, outcome, expiry. Never the token. GitHub's own audit shows what the App did.

### Failure modes (all fail closed)
| Failure | Result |
|---|---|
| Broker or board down | `unavailable`; no token. Session keeps read/comment through the GitHub connector as `baileyrd` |
| Bad agent key, role not allowed, repo not allowed | `denied`, audited |
| GitHub 5xx | one retry, then `upstream_error` |
| GitHub 401 (key revoked or clock skew) | `app_key_rejected`; JWT uses `iat` -60s, `exp` 9 min |
| Config or key missing at start | refuses to start |

Not fixed: the GitHub MCP tools in a cloud session still act as `baileyrd`
(g1-identity.md answer 1). The broker fixes pushes and PRs made with the minted token.
Option a (cloud Claude comments only, never approves) still stands; a reviewer-App token
*can* approve, so that stays a rule the reviewer checks until `gh_proxy` restricts it.

## Smallest viable build
New crate `crates/apps/rusty_token_broker` **in rusty_mill, not a new repo** (entry-rule:
no repo without overlap check; it reuses `rusty_mcp_server`, `rusty_oauth_server`,
`rusty_request`, `rusty_log`, CI and the review flow). Modules, each testable alone:
`policy` (pure: request + identity -> grant or deny), `signer` (port: RS256 JWT),
`github` (mint adapter), `identity` (port: board verify), `audit`, `mcp` (one tool).
v0 = policy + signer + github + audit + a bearer-token identity for one local test, proven
against a throwaway App. v1 = board identity adapter. v2 = task binding, `gh_proxy`.

## Ask-first items for Nano
1. **Host**: BaileyAi (recommended) or elsewhere. No decision covers it.
2. **Signing dependency** (**decided, Nano 2026-11-16, decision card: openssl shell-out**): rusty_rsa only verifies. Options: (a) shell out to `openssl dgst
   -sign` behind the `signer` port, no new dependency, recommended for v0; (b) the `rsa`
   crate, already in Cargo.lock; (c) write private-key signing in `rusty_rsa` (needs
   blinding; most work and risk).
3. **Mill Board verify tool** (read-only: `agent_id`+`key` -> role). A public API change
   queued behind Mill Board's in-flight change (one-change-per-system). v0 avoids it.
4. **Role table** (g1-roles-proposal.md, approved 2026-11-15) and **which App serves which
   role**; Apps exist, roles unmapped.
5. **Option a vs b**: building this is the move from a to b for builder work.

[agent-register-design.md]: agent-register-design.md
