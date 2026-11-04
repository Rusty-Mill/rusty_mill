---
category: Fixed
changelog: The gateway proxy no longer adds rusty_request's User-Agent to a request that had none; rusty_request gains ClientBuilder::no_user_agent.
---
## 2026-11-03 - Gateway proxy forwards a request without a User-Agent as it arrived

- **Added:** `rusty_request::ClientBuilder::no_user_agent()`, which stops the client sending its
  own `User-Agent`. A request still sends one if it sets it.
- **Fixed:** the agentgateway HTTP proxy builds its upstream client with `no_user_agent()`, so a
  proxied request without a `User-Agent` reaches the upstream without one instead of with
  `rusty_request/<version>` (card #409). A client's own `User-Agent` is forwarded unchanged.
