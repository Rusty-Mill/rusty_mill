---
category: Changed
changelog: agentgateway's extAuthz, LLM, moderation, webhook and agent-card clients move from reqwest to rusty_request (#885).
---
## 2026-10-25 - agentgateway: outbound calls on rusty_request

- **Changed:** `agentgateway-auth` (`extAuthz`), `agentgateway-llm` (providers, `promptGuard` moderation and webhooks) and `agentgateway-a2a` (agent cards) call out with `rusty_request` instead of `reqwest`, so none of the three depends on `reqwest` and `agentgateway-llm` no longer depends on `hyper` or `futures-util`. This is the first step of moving the gateway onto `rusty_axum` and `rusty_tokio`: `rusty_request` runs on either runtime. Timeouts, retries (a connect failure or a listed status) and the 32 MiB response cap behave as before. An `ai` route's timeout still covers a streamed answer to its last byte.
- **Known limitations:** `rusty_request` reads only `HTTP_PROXY` from the environment, not `HTTPS_PROXY`, and these clients configure no proxy, so a gateway that reached its providers through an environment proxy needs one set another way. Requests now carry `rusty_request`'s `User-Agent`. Tests still use `reqwest` until they move with the server.
