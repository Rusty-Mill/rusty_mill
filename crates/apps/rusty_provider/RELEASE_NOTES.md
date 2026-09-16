# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

User-facing and operator-facing changes to rusty_provider, one entry per
merged PR against `main`, newest first. No version tags exist yet, so
entries are tracked by PR rather than by release.

---

## PR #149 — Add routing-decision trace headers (X-RP-Decision / X-RP-Fallback-Attempts)
**2026-08-08** · [#149](https://github.com/baileyrd/rusty_provider/pull/149)

- **Added:** `/v1/chat/completions` responses (streaming and
  non-streaming) now carry `X-RP-Decision` (`strategy=<direct|fallback|
  fusion>; provider=...; model=...; latency_ms=...`) and
  `X-RP-Fallback-Attempts` -- which concrete provider/model actually
  served an alias/chain request, and how many candidates it took, without
  a separate `GET /v1/generation?id=` round trip. `provider`/`model`
  reflect the actual dispatched candidate, not just the requested alias.
  Streaming sets both on the initial HTTP response, not as trailers,
  since the winning candidate is known before the first chunk is
  produced.
- `Router::dispatch`/`dispatch_stream` keep their existing signatures;
  each gained a `_traced` sibling (`dispatch_traced`/
  `dispatch_stream_traced`) returning the new `DispatchTrace` alongside
  the response.
- 8 new tests in `rp-router` (strategy classification, attempt counting,
  fusion panel size, cache-hit zero-attempts, streaming trace-before-first-
  chunk) plus 4 full-HTTP-round-trip tests in `rp-server` (direct
  request, route alias, chain fallthrough, streaming); full workspace
  suite passing.

---

## PR #148 — Add provider.max_request_price_usd + budget_fallback per-request cap
**2026-08-08** · [#148](https://github.com/baileyrd/rusty_provider/pull/148)

- **Added:** `provider.max_request_price_usd` caps a single request's
  estimated cost, in USD -- estimated per candidate as
  `max_tokens * completion_per_million` from `[[pricing]]`, a different
  axis from the existing `provider.max_price` (a per-million-token
  ceiling on individual candidates, not a per-request total). Only takes
  effect when the request also sets `max_tokens`. `provider.budget_fallback`
  controls what happens once at least one candidate doesn't fit:
  `"strict"` narrows the chain to just the candidates that do, failing
  with `402` if none do; `"cheapest"` (the default) always serves the
  request -- routing to the cheapest fitting candidate, or, if none fit,
  the overall cheapest candidate anyway.
- 10 new tests in `rp-router` (plus 1 in the `error.rs` status-code
  suite) covering the no-op cases, a generous cap keeping the full chain
  sorted cheapest-first, a cap narrowing the pool, both fallback modes
  when nothing fits, an unpriced candidate treated as ineligible, and two
  end-to-end `dispatch()` tests proving the wiring; full workspace suite
  passing.

---

## PR #147 — Add strategy = "fusion" routing: parallel panel + judge synthesis
**2026-08-08** · [#147](https://github.com/baileyrd/rusty_provider/pull/147)

- **Added:** a new `[[routes]]` dispatch mode, `strategy = "fusion"`,
  alongside the default sequential fallback. `chain` doubles as the
  "panel" -- every entry is dispatched concurrently instead of tried one
  at a time, and a designated `judge` model synthesizes one final answer
  from whichever candidates responded within `fusion_timeout_secs` (each
  panel member independently timed out, so the total wait doesn't scale
  with panel size). Panel answers reach the judge under an anonymized
  `"Candidate 1"`/`"Candidate 2"` label rather than by provider/model. A
  tool-calling or streaming request bypasses fusion entirely and falls
  back to ordinary sequential dispatch, as does a fusion alias with no
  `judge` configured (a startup warning, not a hard failure).
  Usage/cost accounting (`GET /v1/usage`/`GET /metrics`/
  `GET /v1/generation?id=`) covers every contributing panel member plus
  the judge, not just the judge's own call.
- 9 new tests in `rp-router` covering config parsing, panel synthesis
  (including the anonymized-label prompt and summed usage), a slow panel
  member timing out without blocking the request, the tool-call bypass at
  both the request-gate and defense-in-depth level, and full cost/usage
  accounting; full workspace suite passing.

---

## PR #133 — Add rp-cli setup: static config-file rewriting for third-party CLI tools
**2026-08-07** · [#133](https://github.com/baileyrd/rusty_provider/pull/133)

- **Added:** `rp-cli setup` (`list`/`show`/`apply`) rewrites a known
  third-party CLI coding tool's own config file to point its endpoint at
  a running rusty_provider instance -- currently
  [opencode](https://opencode.ai) and [Crush](https://charm.land/crush),
  both verified against their current documented config schemas. The
  target list is data (`crates/cli/cli_targets.toml`), extensible via
  `--targets <path>`, not hardcoded per-tool Rust. `setup show` is a dry
  run; `setup apply` requires `--yes` and always backs up the previous
  file to `<path>.bak` first, merging into whatever's already there
  rather than overwriting it. Never writes a literal API key -- a field
  that needs one gets the target tool's own env-var-reference syntax
  (opencode's `{env:VAR}`, Crush's `$VAR`) naming whatever variable
  `--api-key-env` names.
- **Changed:** `ARCHITECTURE.md`'s non-goal narrows from "no MITM-based
  third-party CLI config injection" to "no traffic interception" -- see
  [ADR-0004](./docs/adr/0004-cli-target-config-rewriting.md). A MITM
  proxy (TLS interception, trust-store changes) is still explicitly out
  of scope and would need its own ADR.
- 22 new tests in `rp-cli` covering the JSON/TOML path-set engine
  (merge-preserves-unrelated-keys, refuses-to-clobber-non-object,
  idempotent rerun, backup-on-apply, `--api-key-env` gating); full
  workspace suite passing.

---

## PR #131 — Add reconnect-with-backoff for dropped MCP gateway upstreams
**2026-08-07** · [#131](https://github.com/baileyrd/rusty_provider/pull/131)

- **Added:** a `[[mcp.upstreams]]` connection that was previously
  established and then drops (upstream process crashes, network blip on
  an HTTP upstream) is now reconnected automatically with exponential
  backoff, instead of staying dead until `rp-server` restarts. New
  `[mcp]` fields: `reconnect_backoff_secs` (default 1),
  `reconnect_backoff_max_secs` (default 60), `max_reconnect_attempts`
  (default unbounded). A startup connection failure is unchanged — still
  a soft warning, absent from the tool list until restart.
- Closes the item explicitly scoped out when MCP support shipped
  (#114): "Reconnect-with-backoff for a dropped upstream (log-and-drop
  is enough to start)."

---

## PR #129 — Add an i18n framework to the dashboard
**2026-08-07** · [#129](https://github.com/baileyrd/rusty_provider/pull/129)

- **Added:** the dashboard's UI text (panel titles, column headers,
  button labels, empty/loading states) now goes through a `t()`-keyed
  translation dictionary and a language switcher in the header,
  persisted in `localStorage`. Framework only — only `en` is populated;
  a new locale is added by extending the dictionary in
  `dashboard.html`, not by guessing a translation nobody asked for.
  Server-generated JSON error messages stay English (a separate,
  larger `Accept-Language`-aware effort, out of scope here).
- **Changed:** `ARCHITECTURE.md`'s non-goals dropped the "no i18n"
  clause, with a note that the dashboard's i18n framework is a
  switching mechanism (English-only today), not a translation project.

---

## PR #126 — Map JWT claims to [[clients]] identity
**2026-08-07** · [#126](https://github.com/baileyrd/rusty_provider/pull/126)

- **Added:** opt-in `[jwt].client_claim` (e.g. `"sub"`) — a verified JWT's
  claim value matched against a configured `[[clients]].name` resolves
  that client's identity for the rest of the request: the same budget
  enforcement, per-subject rate-limit bucket, and usage/spend tracking a
  static per-client API key already gets. No match (claim absent, or no
  client with that name) falls back to the prior behavior unchanged —
  same access a valid `server.api_key_env` token would get, no budget/
  spend tracking, rate-limited via the IP fallback. `/v1/admin/*` is
  untouched by design, `client_claim` included.
- **Changed:** `matched_client_name` replaced by `resolve_client_identity`
  — a request's client identity is now resolved once per request and
  threaded into both rate-limit resolution and dispatch, rather than each
  call site independently re-deriving it from the bearer token.
- Closes the scope explicitly deferred when `[jwt]` shipped (#109/#110):
  "no JWT-claims-to-`[[clients]]`-identity mapping in this pass."

---

## PR #123 — Add a minimal static dashboard at GET /dashboard
**2026-08-07** · [#123](https://github.com/baileyrd/rusty_provider/pull/123) · [ADR-0003](docs/adr/0003-minimal-static-dashboard.md)

- **Added:** `GET /dashboard` — one self-contained HTML file
  (`crates/server/assets/dashboard.html`), no build step, no npm, no JS
  framework, no CDN dependency, compiled into `rp-server` via
  `include_str!`. Renders entirely client-side: prompts for a bearer
  token and attaches it to every `fetch()` against the existing JSON
  endpoints (`/v1/models`, `/v1/usage`, `/v1/providers/stats`,
  `/v1/free-tiers`, `/v1/admin/clients` + per-client usage-history
  sparkline + a reset-spend button), so it's subject to exactly the same
  `check_auth`/`check_admin_auth` rules those endpoints already enforce.
  The page itself is served unauthenticated (it carries no secrets), same
  reasoning as `/health`.
- **Changed:** `ARCHITECTURE.md`'s non-goal softened again, from "no UI"
  to "no Electron/PWA/desktop product" — see ADR-0003, superseding
  [ADR-0002](docs/adr/0002-reporting-surface-is-json-only.md).
- Zero new Rust dependencies, zero new config surface, zero new persisted
  state — a rendering layer over what already existed.

---

## PR #121 — Audit SSE streaming coverage; add unrecognized-field robustness tests
**2026-08-07** · [#121](https://github.com/baileyrd/rusty_provider/pull/121)

- **Added:** unrecognized-extra-field robustness tests for the Gemini and
  OpenAI-compatible streaming adapters (`WireResponse`/`WireChunk` and
  friends rely on serde's default "ignore unknown fields" behavior, but
  neither had a test proving it). Anthropic already had equivalent
  coverage via its explicit event-type discriminator.
- **Known limitation (confirmed, not a gap):** SSE byte-to-event framing
  (a JSON payload split across reads, comment/keep-alive lines) is
  `eventsource-stream`'s own tested responsibility, fully resolved before
  any adapter sees a complete event — audited and found not to need
  application-level tests. Closes #80.

---

## PR #120 — Add historical/time-series usage export admin endpoint
**2026-08-07** · [#120](https://github.com/baileyrd/rusty_provider/pull/120)

- **Added:** `GET /v1/admin/clients/{name}/usage-history?days=N` —
  day-bucketed `requests`/`prompt_tokens`/`completion_tokens`/`cost_usd`
  for a client, oldest first, over the last `N` days (default 30, capped
  at 90). New `client_daily_usage` table in both the SQLite and Postgres
  persistence backends. Applies to every named client, not just ones with
  a configured budget — history is a different concern from budget
  enforcement. No-op (empty `data`) without `[persistence]` configured,
  since history needs to survive a restart to mean anything.

---

## PR #119 — Make auto-routing tier resolution cost-aware
**2026-08-07** · [#119](https://github.com/baileyrd/rusty_provider/pull/119)

- **Changed:** when `model: "auto"` resolves to a `[[routes]]` alias
  spanning multiple candidates, dispatch now defaults `provider.sort` to
  `"price"` among them, unless the request already set its own explicit
  `sort` (which always wins unchanged). Previously the complexity-based
  tier classifier had no visibility into per-model pricing at all.

---

## PR #118 — Automatically deprioritize unhealthy providers in chain resolution
**2026-08-07** · [#118](https://github.com/baileyrd/rusty_provider/pull/118)

- **Changed:** chain resolution now stably deprioritizes (not
  re-ranks) any candidate with an observed EWMA success rate below
  `0.5`, by default. Previously that health signal only applied when a
  request explicitly opted in with `sort: "uptime"`; it's skipped when
  that sort is explicitly requested, since it's already the fuller
  version of the same concern.

---

## PR #117 — Retry a transient error against the same provider before falling through
**2026-08-06** · [#117](https://github.com/baileyrd/rusty_provider/pull/117)

- **Added:** one same-candidate retry (fixed 200ms backoff) on a
  genuinely transient error (timeout, network error, `5xx`) before
  falling through to the next chain entry, across `dispatch`/
  `dispatch_stream`/`embeddings`. `ProviderError::is_transient()` — a
  strict subset of `is_retryable()` — excludes rate limits and
  unsupported-content/feature mismatches, since retrying the *same*
  candidate can't fix either.

---

## PR #116 — Add structured audit log for admin API mutations
**2026-08-06** · [#116](https://github.com/baileyrd/rusty_provider/pull/116)

- **Added:** a structured `tracing::info!` "admin action" event
  (identity, organization, action, target) on every successful
  `admin_create_client`/`admin_update_client`/`admin_delete_client`/
  `admin_reset_client_spend` mutation — previously no admin mutation was
  logged anywhere distinct from normal request tracing.

---

## PR #115 — Warn on unresolvable route-alias providers; add global concurrency cap
**2026-08-06** · [#115](https://github.com/baileyrd/rusty_provider/pull/115)

- **Added:** `Router::from_config` now warns at startup when a
  `[[routes]]` alias's chain references a provider name with no matching
  `[[providers]]` entry, instead of only surfacing the typo implicitly
  through degraded fallback behavior at request time.
- **Added:** `server.max_concurrent_requests` — a server-wide in-flight
  request ceiling (`Semaphore::try_acquire_owned`, enforced as the
  outermost middleware layer); once saturated, the next request gets
  `503` immediately rather than queuing. Distinct from the existing
  per-caller rate limiting, which bounds rate, not total in-flight count.
  Unset by default (no cap).

---

## PR #114 — Add MCP support: expose rusty_provider as an MCP server and gateway
**2026-08-06** · [#114](https://github.com/baileyrd/rusty_provider/pull/114) · [docs/MCP.md](docs/MCP.md)

- **Added:** `[mcp]` config section, opt-in. Two directions at once, built
  on [`rusty_mcp`](https://github.com/baileyrd/rusty_mcp): rusty_provider's
  own routing exposed as MCP tools (`chat_completion`/`list_models`/
  `embeddings`), plus a gateway proxying configured `[[mcp.upstreams]]`
  (stdio subprocess or Streamable HTTP) under `"{upstream}/{tool}"` names,
  merged into one `tools/list`. Mounted inside the existing app/port,
  guarded by the same `server.api_key_env`/`[[clients]]`/`[jwt]` auth every
  other route already uses, not a separate auth model. `MCP_STDIO=1` serves
  the same handler over stdio for desktop clients. New dependencies
  `rusty-mcp` (git) and `rmcp`.

---

## PR #111 — rp-cli: cover [jwt].hs256_secret_env in keys check / config check
**2026-08-06** · [#111](https://github.com/baileyrd/rusty_provider/pull/111)

- **Fixed:** `rp-cli keys check` silently omitted `jwt.hs256_secret_env`
  (added after `rp-cli` itself was written) from its audit, and
  `config check` never reported JWT status at all. Found via a live
  smoke-test of the running server rather than the existing test suite.
  Both now report it, mirroring the existing admin-API status line.

---

## PR #110 — Add [jwt] JWT/OIDC bearer-token authentication
**2026-08-06** · [#110](https://github.com/baileyrd/rusty_provider/pull/110)

- **Added:** `[jwt]` config — JWT/OIDC bearer-token auth, additive
  alongside `server.api_key_env`/`[[clients]]`. `hs256_secret_env`
  (shared secret) or `jwks_url` (RS256, cached by `kid`), optional
  `issuer`/`audience` validation. Fails closed on any verification
  failure; the validation algorithm is always chosen by this router's
  own configured mode, never trusted from the token's own `alg` header.
  Follow-up to the OmniRoute/agentgateway parity-loop run (#100) — closes
  #109. No JWT-claims-to-client-identity mapping in this pass (documented
  as out of scope).

---

## PR #100 — Parity-loop: close additive capability gaps vs. OmniRoute/agentgateway
**2026-08-06** · [#100](https://github.com/baileyrd/rusty_provider/pull/100)

- **Changed:** `ARCHITECTURE.md`'s non-goals softened from "no dashboard" /
  "not multi-tenant SaaS" to "no UI" — a JSON-only reporting surface is in
  scope, an HTML/Electron/PWA dashboard is not. See
  [ADR-0002](docs/adr/0002-reporting-surface-is-json-only.md).
- **Added:** `docs/PROVIDERS.md` — a curated reference table of ~20 more
  OpenAI-wire-compatible backends (Mistral, Cerebras, SambaNova, DeepSeek,
  OpenRouter, Hugging Face, NVIDIA NIM, Novita, DeepInfra, Nebius, Moonshot,
  Zhipu, DashScope/Qwen, xAI, Perplexity, Cohere, Hyperbolic, Featherless,
  01.AI, Cloudflare Workers AI), plus matching commented-out presets in
  `config.example.toml`. All config-only — `kind = "openai"` already covers
  any of them, same as Groq/Together/Fireworks today.
- **Added:** `[[free_tiers]]` config + `GET /v1/free-tiers` — operator-
  declared free-token budgets per "provider/model", tracked against this
  router's own usage and reported (budget/used/remaining) the same
  reporting-only, self-declared way `zdr`/`no_training` already work.
  Reset cadence reuses `[[clients]].budget_period`'s calendar math
  (`"total"`/`"daily"`/`"weekly"`/`"monthly"`, default `"monthly"`).
- **Added:** Three new `provider.sort` strategies — `"quality"` (descending
  by a new operator-declared `[[pricing]].quality_score`), `"random"`
  (shuffles the chain for simple load distribution, no new dependency —
  a tiny in-crate splitmix64 PRNG), and `"free_tier_remaining"` (descending
  by headroom against the `[[free_tiers]]` budgets above).
- **Added:** `transforms: ["rtk"]` — tool-output compression alongside the
  existing `"middle-out"`. A built-in, content-sniffed 5-category filter
  catalog (git/test/build/package/generic) compacts `role: "tool"` message
  text before dispatch; composes with `"middle-out"` when both are set.
- **Added:** `rp-cli` — a new 5th workspace crate, a synchronous read-only
  operator CLI (`config check`/`providers list`/`keys check`) built
  directly on `rp-router::Config`, so it can never drift from the schema
  the real server loads. Not built into the Docker image.
- **Added:** `[cache].mode = "semantic"` — opt-in alongside the existing
  exact-match caching, embedding-cosine-similarity matching on message
  text only (every other field still has to match exactly). Embeds via
  this router's own `/v1/embeddings` dispatch path
  (`[cache].embedding_model`); falls back to exact-match at startup with
  a warning if that model doesn't resolve. Fails open on an embedding-
  call failure, same as Moderation's own backend-failure handling.

---

## PR #98 — Update ARCHITECTURE.md's stale caching claims
**2026-07-22** · [#98](https://github.com/baileyrd/rusty_provider/pull/98)

- **Fixed:** `ARCHITECTURE.md` (added by an earlier repo-config pass)
  predated the opt-in response cache from #65/#86 and still listed *"no
  response cache today"* as a non-goal. Now documents the cache in the
  `rp-router` structure bullet and the dispatch data-flow step (a hit
  is checked first and skips chain resolution/dispatch/usage-recording
  entirely), and narrows the non-goal to what's still true: exact-match
  only, no semantic/fuzzy matching.

---

## PR #96 — Add cargo-audit CI job; drop prometheus's unused protobuf feature
**2026-07-22** · [#96](https://github.com/baileyrd/rusty_provider/pull/96)

- **Added:** `.github/workflows/audit.yml` runs `cargo audit` against
  `Cargo.lock` on every push/PR touching a `Cargo.toml`/`Cargo.lock`,
  plus daily on a schedule — a newly published advisory against an
  already-pinned dependency can't go unnoticed between pushes.
- **Fixed:** a real, live advisory —
  [RUSTSEC-2024-0437](https://rustsec.org/advisories/RUSTSEC-2024-0437)
  (uncontrolled recursion, crash) in `protobuf` 2.28.0, pulled in
  transitively via `prometheus`'s default `protobuf` feature. Only
  `TextEncoder` (Prometheus text-exposition format) is ever used here,
  never the protobuf wire format, so `prometheus` now builds with
  `default-features = false` — drops the dependency (and the advisory)
  entirely, no functional change.

---

## PR #94 — Add MIT LICENSE file
**2026-07-22** · [#94](https://github.com/baileyrd/rusty_provider/pull/94)

- **Fixed:** `Cargo.toml` has declared `license = "MIT"` since the
  workspace's first commit, but the license text itself was never
  reproduced anywhere in the repo — a real compliance gap for anyone
  consuming the crate or repo. Adds a standard MIT `LICENSE` file and a
  README "License" section linking to it.

---

## PR #92 — Add multi-stage Dockerfile for container deployment
**2026-07-22** · [#92](https://github.com/baileyrd/rusty_provider/pull/92)

- **Added:** a multi-stage `Dockerfile` (+ `.dockerignore`) producing a
  slim `debian:bookworm-slim` runtime image for `rp-server`. Uses
  `cargo-chef`, built from the official `rust:1-bookworm` image, to split
  dependency compilation from the workspace's own source, so a
  source-only edit doesn't force `ring`/`rusqlite`/`tokio-postgres` and
  the rest of the dependency graph to recompile.
- Runtime installs `ca-certificates` explicitly, since
  `rustls-native-certs` (outbound provider TLS, and an optional
  TLS-enabled `[persistence]` Postgres connection) reads the OS trust
  store at runtime, not just at build time. Runs as a non-root user;
  ships a `HEALTHCHECK` against `/health`.
- Nothing secret is baked in — `config.toml` and provider API keys are
  supplied at `docker run` time (bind-mount + env vars), documented in a
  new README "Docker" section.
- Added a `docker build` CI job for ongoing verification.

---

## PR #90 — Add GET /ready readiness check, distinct from /health
**2026-07-22** · [#90](https://github.com/baileyrd/rusty_provider/pull/90)

- **Added:** `GET /ready`, distinct from the existing `GET /health`.
  `/health` stays a cheap, unauthenticated liveness check that never
  touches anything external. `/ready` actually confirms the router can
  serve traffic: when `[persistence]` is configured, a trivial round
  trip against its database, returning `503` with a reason if that
  fails. Without `[persistence]` there's nothing external to check, so
  `/ready` is always `200`, same as `/health`.
- Point an orchestrator's readiness probe at `/ready` and its liveness
  probe at `/health` — a `503` from `/ready` should pull an instance out
  of rotation without restarting it, since the process itself is fine.
- No new config knobs; reuses the existing `[persistence]` section.

---

## PR #88 — Add configurable request body size limit
**2026-07-22** · [#88](https://github.com/baileyrd/rusty_provider/pull/88)

- **Added:** `server.max_body_bytes`, applied as a `DefaultBodyLimit`
  layer over the whole router, defaulting to 20 MiB. Rejected requests
  get `413 Payload Too Large` before a handler ever parses the body.
- **Fixed:** axum's `Json`/`Bytes` extractors already enforced an
  implicit 2 MB body limit even without this config, but that ceiling
  was neither explicit nor operator-configurable, and was tight enough
  to reject a legitimate multimodal request — an inline
  base64-encoded image, audio clip, or PDF adds ~33% overhead over the
  original file's size. `max_body_bytes` replaces that implicit
  ceiling rather than adding a second one on top of it.
- Applies globally, not only to `/v1/chat/completions`.

---

## PR #86 — Add opt-in response cache for identical requests
**2026-07-22** · [#86](https://github.com/baileyrd/rusty_provider/pull/86)

- **Added:** `[cache]`, an opt-in, in-memory, exact-match cache of
  non-streaming `/v1/chat/completions` responses, keyed by a hash of the
  entire incoming request. Fully off (no overhead) unless `[cache]` is
  configured. Entries expire after `ttl_secs` (default 300) and the
  cache holds at most `max_entries` (default 1000), evicting the
  oldest entry once over capacity — the same eviction strategy
  `GET /v1/generation?id=` already uses.
- **Known limitation:** exact-match only, no semantic/fuzzy matching —
  any difference in the request (model, messages, sampling parameters,
  provider preferences) is a cache miss.
- **Known limitation:** streaming requests always bypass the cache in
  both directions; caching a replayed SSE chunk sequence is left for a
  future version.
- A cache hit skips dispatch to the provider and skips re-recording
  usage/cost/latency/throughput/generation-cache bookkeeping for that
  request, since it already ran once when the response was first
  computed — this keeps `/v1/usage` and `/metrics` from double-counting
  a single real generation. New `rusty_provider_cache_lookups_total`
  Prometheus counter, labeled `hit`/`miss`.
- Not the same thing as `cache_read_per_million`/`cache_write_per_million`
  or `cache_control` (see [Prompt caching](README.md#prompt-caching)),
  which price a provider's own prompt-cache discount rather than a
  router-side response cache.
- 18 new unit tests across `rp-router` (`cache.rs`, `config.rs`,
  `metrics.rs`, and `dispatch`-level cache hit/miss/bypass behavior).

---

## PR #84 — Add POST /v1/embeddings endpoint
**2026-07-21** · [#84](https://github.com/baileyrd/rusty_provider/pull/84)

- **Added:** `POST /v1/embeddings`, OpenAI-compatible request/response
  shape. Implemented by the OpenAI-compatible adapter (direct
  passthrough) and Gemini (via `batchEmbedContents`, used even for a
  single input to avoid a second wire shape). `Router::embeddings`
  reuses `dispatch`'s chain-resolution and retryable-error fallback.
- **Known limitation:** Anthropic has no embeddings API at all, so it
  always returns a retryable `UnsupportedFeature` error — a chain
  naming it alongside a real embeddings provider falls through rather
  than failing, but it can never itself serve an embeddings request.
- **Known limitation:** none of `[[presets]]`, `[[guardrails]]`,
  `[moderation]`, `[web_search]`, or spend budgets apply to this
  endpoint yet — only auth and inbound rate-limiting, same as
  `/v1/chat/completions`'s auth layer. Cost/latency/throughput
  tracking also don't apply, since there's no established pricing
  shape for a prompt-only, no-completion-tokens request yet.
- 20 new/updated unit and integration tests across `rp-core`,
  `rp-providers`, `rp-router`, and `rp-server`; full suite passing.

## PR #83 — Add standard governance/docs scaffold
**2026-07-21** · [#83](https://github.com/baileyrd/rusty_provider/pull/83)

- **Added:** `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`,
  `ARCHITECTURE.md`, `CHANGELOG.md`, this file, PR/issue templates, and
  an ADR log seed (`docs/adr/0001-template.md`), via the repo-config
  skill. `ARCHITECTURE.md`'s boundary table and structure sections are
  filled in for real (the `Provider` trait, the 3 adapters, the
  persistence backend port, the request-dispatch data flow), not left
  as scaffold.
- **Known limitation:** the skill's default CI template
  (`ci-rust.yml`) was dropped rather than added, since this repo
  already has a working `.github/workflows/ci.yml` (fmt/clippy/test
  plus a Postgres service for the `[persistence]` backend's tests) —
  adding a second, less-tailored "CI" workflow would have run
  redundant, weaker checks on every push.
