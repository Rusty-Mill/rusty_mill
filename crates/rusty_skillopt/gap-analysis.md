# Gap analysis: rusty_skillopt vs. Microsoft SkillOpt

Reference: Microsoft's SkillOpt (Python), as described in this repo's own
README "Scope" section plus earlier research in this session (a WebFetch
summary of the upstream README — not full source access, so treat feature
descriptions as directional, not authoritative).

This isn't a `cargo public-api` symbol diff — the reference is a different
language and a CLI/training-loop product, not a Rust crate with an exported
API surface. Rows are feature-area gaps, judged by hand against this repo's
existing `ChatBackend`/`Environment` trait boundaries and README Scope list.

| Item | Category | Reference | Breaking? | Est. size | Autonomous? | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| Azure OpenAI auth | backend | SkillOpt's Azure backend | no | S | yes | Azure OpenAI uses `api-key` header (not Bearer) and a different URL shape (`https://{resource}.openai.azure.com/openai/deployments/{deployment}/...?api-version=...`) than plain OpenAI-compatible. Doesn't fit the current `openai_compatible` provider's request shape as-is; well-documented, stable API, no new dependency expected (reqwest+serde like existing backends). |
| Qwen backend | backend | SkillOpt's Qwen backend | no | S | yes | Alibaba's DashScope has an OpenAI-compatible mode — likely already coverable by the existing `openai_compatible` provider via `base_url`, same as the Ollama config just added, with **no new code**. Issue is to verify this concretely (example config + a documented note) rather than assume a new backend is needed; falls back to a small new backend only if compat mode turns out insufficient. |
| MiniMax backend | backend | SkillOpt's MiniMax backend | no | M | **needs-human** | Uncertain whether MiniMax's API is OpenAI-compatible or fully proprietary — I don't have reliable, current knowledge of its exact request/response shape, and verifying by fetching MiniMax's docs may hit the same egress-policy wall that blocked `ollama.com` in this session. Needs a human to either confirm the API shape or confirm the docs domain is reachable before this can be implemented confidently. |
| WebUI / monitoring dashboard | subsystem | `skillopt_webui` (Gradio) | no | L | **needs-human** | Whole new subsystem, not a small addition. Needs a new third-party dependency (an HTTP server/web framework — nothing in this workspace serves HTTP today) and real design decisions (what state to expose, polling vs. push, single-binary vs. separate crate). Per the loop's own rules, a new dependency is a stop-and-ask regardless of size. |
| Offline "Sleep" engine | subsystem | `skillopt_sleep` (v0.2.0+) | no | L | **needs-human** | Harvests agent sessions, mines patterns, replays tasks, consolidates validated skills, with integration shells for Claude Code/Codex/Copilot/Devin. Genuine design ambiguity (session log format, which integration first, storage) beyond what a single small issue can resolve — needs a scoping conversation, not just a dependency sign-off. |
| Real benchmark adapter (e.g. SearchQA) | benchmark | SkillOpt's `envs/searchqa/` | no | M | **needs-human** | Pure addition against the existing `Environment` trait (same shape as `synthetic_arithmetic`), so code-wise it's loop-shaped. But it needs a real dataset from an external host, and this session already hit an org egress-policy 403 on an unlisted domain (`ollama.com`) — the dataset host may be blocked the same way. Needs a human to confirm the dataset is fetchable (or vendor a small fixture) before implementation starts. |

## Recommendation for this round

Only **Azure OpenAI auth** and **Qwen backend** are clear autonomous-loop
candidates: small, additive, no new dependency, well-defined. I'll file all
six as issues so everything is tracked, but label the other four
`needs-human` up front so the loop skips them rather than stalling on each
one individually mid-run.
