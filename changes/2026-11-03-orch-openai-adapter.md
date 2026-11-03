---
category: Added
changelog: orch-openai sends one OpenAI-compatible chat completion over plain HTTP to loopback and private addresses only, for llama-swap in the lens runner.
---
## 2026-11-03 - rusty_orch: orch-openai adapter for local model servers

- **Added:** `orch-openai`, a blocking client for `POST {base_url}/chat/completions` on the workspace's `rusty_http`. `https` is refused. Every call resolves the host, refuses unless every address is loopback, RFC 1918 or `fc00::/7`, and connects to an address it checked. One deadline bounds the call. Errors are classified for fallback (unreachable or unauthorised is `Unavailable`; timeout, 429 or 5xx is `Transient`). An optional key is read from its environment variable at call time and masked out of errors. Let Em Loose #594, ADR-0020.
- **Known limitations:** no TLS, so no Gemini yet. An IPv6 literal URL needs an explicit port. The crate returns text and usage; the lens runner's parser validates replies in a later PR. Nothing uses it yet.
