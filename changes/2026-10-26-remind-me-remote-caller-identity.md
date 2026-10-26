---
category: Fixed
changelog: remind_me_remote stamps each write with the calling client and sends the server's usage instructions at initialize.
---
## 2026-10-26 - Remote connector names its callers and sends its instructions

- **Fixed:** memories written through the remote connector (claude.ai and other HTTP clients) were stamped `client=unknown`, because the handler ignored the caller in each call's context. Each tool call now carries its caller (`name/version` from that session's `initialize`) in the request's `_meta` under `rusty-remind-me/caller`, and the MCP server runs the call as that caller, held only for the call. It holds through the daemon, where every remote caller shares one connection, and concurrent remote clients never share a process-wide slot (ADR-0028 step 1).
- **Fixed:** the remote server now sends the same `instructions` at `initialize` as the stdio server, so remote clients get the usage rules too.
- **Known limitations:** the client name is advisory, as on stdio: the client supplies it and nothing verifies it.
