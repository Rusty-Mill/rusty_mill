# gap-analysis.md — rusty_provider vs. OmniRoute

Run scope settled 2026-08-06 (see chat): OmniRoute (diegosouzapw/OmniRoute) is a
full TypeScript product (Electron desktop app, PWA, 43-language i18n,
dashboard, 291 providers) built around aggregating documented free-tier LLM
quotas behind one OpenAI-compatible endpoint. rusty_provider is a headless
Rust HTTP router; its `ARCHITECTURE.md` currently states "not a full LLM
gateway UI/analytics product, no dashboard" and "not multi-tenant SaaS" as
explicit non-goals.

User's scope decision for this run:
- Parity target: revise those two non-goals so a JSON-only reporting surface
  and richer routing/config surface are in scope — **not** literal product
  parity (no Electron app, no PWA, no 43-language i18n, no MITM-based
  coding-tool config injection). Those stay explicitly out of scope.
- Provider breadth: don't chase OmniRoute's raw 291-provider count. Confirm
  the existing `[providers.X]` config is already provider-count-agnostic
  (`kind = "openai"` covers any OpenAI-wire-compatible backend today — Groq/
  Together/Fireworks already prove this), then add a curated batch of
  documented free/high-value OpenAI-compatible endpoints as ready-to-uncomment
  config presets + a reference doc, rather than one adapter per provider.
- In-scope differentiators (user-selected): token/output compression,
  free-tier tracking endpoint, more routing strategies, an operator CLI.

Source for all rows below: **spec** (read directly from OmniRoute's own docs —
no `cargo public-api`-diffable surface exists between a Rust workspace and a
TypeScript monorepo, and rusty_provider has no pre-existing ROADMAP.md to
audit against).

## Explicitly out of scope this round

Noted so a later run doesn't rediscover these as "missing": Electron desktop
app, PWA/service worker, 43-language i18n, MITM-based third-party CLI config
injection (`omniroute setup-*`), ACP agent spawning, account-rotation/"combo"
multi-key-per-provider pooling (OmniRoute's reset-aware/quota-share routing
depends on this and rusty_provider has no multi-account-per-provider concept
today), literal 291-provider adapter count, marketing site/Discord/sponsor
infrastructure. Also explicitly declined: reproducing OmniRoute's free-tier
*aggregation* model as-is — many of its 220+ tracked providers' own ToS
explicitly prohibit proxy/resale use (see `docs/reference/FREE_TIERS.md`'s ToS
table in OmniRoute); rusty_provider's version is scoped as an **operator
self-declared** budget report (matching the existing `zdr`/`no_training`
trust model), not a service that aggregates or launders other providers'
free tiers on the operator's behalf.

## Gaps

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `ARCHITECTURE.md` non-goals | docs | spec | n/a | user scope decision | no | S | Soften "no dashboard"/"not multi-tenant SaaS" to reflect the new reporting endpoint below; record as an ADR. Prerequisite for the rest — do first. |
| Curated provider presets | docs/config | spec | n/a | `docs/reference/PROVIDER_REFERENCE.md`, `public/providers/*` | no | M | `config.example.toml` already supports any OpenAI-wire backend via `kind = "openai"`. Add ~20-25 commented-out presets (Mistral, Cerebras, SambaNova, DeepSeek, OpenRouter, Cloudflare Workers AI, HuggingFace router, NVIDIA NIM, Novita, DeepInfra, Nebius, Moonshot/Kimi, Zhipu/GLM, Alibaba Qwen/DashScope, 01.AI/Yi, xAI/Grok, Perplexity) with base_url + known free-tier notes, plus a `docs/PROVIDERS.md` reference table. |
| `[[free_tiers]]` config + `GET /v1/free-tiers` | fn (new) | spec | n/a | `docs/reference/FREE_TIERS.md` | no | M | Operator-declared monthly free-token budget per provider/model (self-declared like `zdr`, never verified against upstream). New endpoint reports configured budget vs. this process's tracked usage (reuses existing usage accounting) — remaining budget, not live scraped quota. |
| `sort: "quality"` | fn (existing, new variant) | spec | n/a | OmniRoute `docs/routing/*` general routing-strategy set | no | S | New operator-declared `quality_score` field on `[[pricing]]`; sorts candidates descending. Purely additive arm in `Router`'s existing `sort` match. |
| `sort: "random"` | fn (existing, new variant) | spec | n/a | — | no | S | Weighted-random ordering across the resolved chain, for simple load distribution instead of deterministic ranking. Additive arm. |
| `sort: "free_tier_remaining"` | fn (existing, new variant) | spec | n/a | OmniRoute reset-aware routing (docs/guides/FEATURES.md) | no | S | Depends on the `[[free_tiers]]` gap above. Prefers the candidate with the most configured-budget headroom left this period. |
| `transforms: ["rtk"]` tool-output compression | fn (new) | spec | n/a | `docs/compression/RTK_COMPRESSION.md` | no | L | New opt-in transform (alongside existing `middle-out`) that runs a built-in filter catalog over `role: "tool"` message content before dispatch — strip ANSI, collapse duplicate lines, condense git/test/build/package-manager/docker output. Mirrors the existing context-compression opt-in pattern; MVP covers 5 filter categories, not OmniRoute's full 49. |
| `rp-cli` operator CLI | fn (new crate) | spec | n/a | `docs/reference/CLI-TOOLS.md` (scoped down) | no | M | New `rp-cli` binary: `config check` (validate `config.toml` parses + report which providers/clients are active), `providers list` (resolved providers + skip reasons), `keys check` (which `api_key_env` vars are set, no values printed). Not OmniRoute's MITM/ACP-spawn CLI — pure config/ops tooling. |

Total: 8 issues, all additive (no `breaking-change` label needed), no new
third-party dependencies anticipated beyond what's already in the workspace
(the `rtk` transform and `rp-cli` are pure Rust/std + existing crates).

## Additional reference: agentgateway.dev

Cross-checked against [agentgateway](https://agentgateway.dev/) — also
Rust-based, a closer architectural peer than OmniRoute (unified gateway for
HTTP/gRPC/MCP/A2A traffic, not just OpenAI-shaped chat completions).

**Already covered, no new gap:** model/cost/latency-aware routing
(`provider.sort`), token budgets (`[[clients]].budget_usd`), per-request
cost calculation (`cost_usd`), team/user cost attribution
(`organization`/`workspace` on `[[clients]]`, `GET /v1/admin/organizations`
— agentgateway's "virtual scoped keys" equivalent), prompt
redaction/blocking (`[[guardrails]]` — regex-based, not agentgateway's
NER-style "PII-shield," but same slot), OpenTelemetry-adjacent observability
(`GET /metrics` Prometheus, per-provider stats).

**Identified but not filed as parity-gap issues** — both cross the skill's
own stop-and-ask line (new protocol surface / new third-party dependency).
User declined both for this run (2026-08-06): stays chat-completions-only,
static-key auth. Left here for a future, separately-scoped run to pick up:

- **MCP (Model Context Protocol) gateway support.** agentgateway proxies MCP
  tool calls with per-identity tool scoping; rusty_provider only speaks
  OpenAI-shaped chat completions today. This is a second protocol surface,
  not an additive endpoint — new dependency, new request/response shapes,
  real design surface (transport: stdio vs. SSE vs. streamable-HTTP; how
  tool scoping composes with existing `[[clients]]` auth).
- **JWT/OIDC authentication.** Today's auth is a static bearer token
  (`server.api_key_env` / `[[clients]].api_key_env`) checked by string
  equality — no token verification library, no IdP/JWKS fetching. Adding
  JWT/OIDC as an alternative auth mode needs a new dependency
  (`jsonwebtoken` + JWKS fetching) and a decision on how it composes with
  existing client/budget/rate-limit identity.

One additional gap **was** filed since it's additive and dependency-free
(the router can already call an embeddings provider itself):

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Semantic response cache | fn (existing, new mode) | spec | n/a | agentgateway.dev "semantic caching" | no | M | Opt-in alongside the existing exact-match `[cache]`: embed the request via the already-configured embeddings provider, cosine-similarity match against cached entries above a configurable threshold. No new dependency — reuses the router's own `/v1/embeddings` dispatch path. |
