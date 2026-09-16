# Provider reference

`config.example.toml` ships six providers wired up by default (OpenAI,
Anthropic, Gemini, Groq, Together, Fireworks) plus a commented-out block of
curated presets. This doc is the fuller reference for that preset block: what
each one is, and a one-line note on why you might reach for it.

Adding any of these is a config change only — `kind = "openai"` covers any
backend that speaks OpenAI's `/chat/completions` wire format, which is true
of every provider below (see `ARCHITECTURE.md`'s `Provider` trait boundary
for why one adapter serves all of them, same as Groq/Together/Fireworks
already do). A provider needing a genuinely different wire format (like
Anthropic's Messages API or Gemini's `generateContent`) needs a new adapter
in `rp-providers`, not a preset — none of the providers below need that.

None of the notes here are a live feed — provider free tiers, pricing, and
ToS change on their own schedule. Confirm current terms on the provider's
own site before relying on anything below, the same caveat this repo's
README already gives for `[[pricing]]` and rate limits.

| Provider | `kind` | `base_url` | Note |
| --- | --- | --- | --- |
| Mistral | openai | `https://api.mistral.ai/v1` | Mistral's own hosted API; documented free tier on signup. |
| Cerebras | openai | `https://api.cerebras.ai/v1` | Wafer-scale inference hardware — notably high tokens/sec; free tier available. |
| SambaNova | openai | `https://api.sambanova.ai/v1` | Free tier for a rotating set of open-weight models. |
| DeepSeek | openai | `https://api.deepseek.com/v1` | Low-cost hosted DeepSeek models; off-peak discount pricing on some plans. |
| OpenRouter | openai | `https://openrouter.ai/api/v1` | An aggregator itself (many providers behind one key) — using it here nests one router inside another; useful mainly for its own `:free`-suffixed model pool. |
| Hugging Face (router) | openai | `https://router.huggingface.co/v1` | Routes across HF Inference Providers; free-tier request volume depends on account tier. |
| NVIDIA NIM | openai | `https://integrate.api.nvidia.com/v1` | build.nvidia.com's hosted NIM endpoints; free API credits on signup, evaluation-scoped per NVIDIA's own terms. |
| Novita | openai | `https://api.novita.ai/v3/openai` | Hosted open-weight models, small free credit on signup. |
| DeepInfra | openai | `https://api.deepinfra.com/v1/openai` | Pay-per-token open-weight hosting, generally cheap; no persistent free tier at time of writing. |
| Nebius (AI Studio) | openai | `https://api.studio.nebius.ai/v1` | Nebius's hosted inference studio; signup credits, not a recurring free tier. |
| Moonshot AI (Kimi) | openai | `https://api.moonshot.ai/v1` | Kimi models; check current region (`.ai` vs `.cn`) for your account. |
| Zhipu / Z.AI (GLM) | openai | `https://api.z.ai/api/paas/v4` | GLM-family models; some GLM-Flash variants have been offered free — verify current status. |
| Alibaba DashScope (Qwen) | openai | `https://dashscope.aliyuncs.com/compatible-mode/v1` | Qwen models via DashScope's OpenAI-compatible mode. |
| xAI (Grok) | openai | `https://api.x.ai/v1` | Grok models, paid API (no persistent free tier at time of writing). |
| Perplexity | openai | `https://api.perplexity.ai` | Perplexity's `sonar` models, including built-in web search variants — no `[web_search]` config needed for those specific models. |
| Cohere | openai | `https://api.cohere.ai/compatibility/v1` | Cohere's OpenAI-compatibility endpoint (their native API is otherwise a different shape). |
| Hyperbolic | openai | `https://api.hyperbolic.xyz/v1` | Open-weight model hosting; signup credits. |
| Featherless AI | openai | `https://api.featherless.ai/v1` | Subscription-based access to a very large open-weight model catalog rather than a free tier. |
| 01.AI (Yi) | openai | `https://api.lingyiwanwu.com/v1` | Yi-family models; confirm current pricing (free tiers for hosted LLM APIs are frequently promotional and get retired). |
| Cloudflare Workers AI | openai | `https://api.cloudflare.com/client/v4/accounts/ACCOUNT_ID/ai/v1` | Per-account URL (replace `ACCOUNT_ID`); free tier is Cloudflare's own Workers AI included-usage allowance, not a separate signup. |

## ToS note

Several providers' terms restrict API-key sharing, reselling, or running the
key behind a proxy for others — worth reading before you put one of these
behind `rusty_provider` for anything beyond your own personal/single-tenant
use, especially if you plan to hand out `[[clients]]` keys to other people.
This repo doesn't track or enforce any of that (same as it doesn't verify
`zdr`/`no_training` against the provider) — it's on you as the operator,
same as with the six providers `config.example.toml` ships by default.
