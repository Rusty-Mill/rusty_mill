---
category: Added
changelog: orch-openai serves the lens runner's Model port; every reply is checked by the lens parser.
---
## 2026-11-03 - rusty_orch: orch-openai answers lens questions

- **Added:** `OpenAiHttp` implements `orch_lens::Model` (Let Em Loose #594). The question's prefix goes as the system message, its body as the user message, its output cap as `max_tokens`. `answer` checks the reply with `orch_lens::parse`; a non-conforming reply is `Permanent`, so nothing unvalidated reaches the board. A reported cached-token count above the input count is held to the input count.
- **Known limitations:** nothing calls it yet; the `rusty_orch lens` shell and the Claude and Codex `Model` impls come in later PRs.
