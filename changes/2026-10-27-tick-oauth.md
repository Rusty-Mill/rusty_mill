---
category: Added
changelog: "rusty_tick: OAuth 2.1 sign-in for `/mcp` (claude.ai \"Sign in now\") when `RUSTY_TICK_ISSUER` is set; grants act as the user whose token approved them, and existing bearer tokens keep working."
---
## 2026-10-27 - OAuth sign-in for rusty_tick's MCP server

- **Why:** a claude.ai custom connector signs in with OAuth; until now rusty_tick only took a pasted bearer token. Second of three PRs after `rusty_oauth_server` (#1179); Mill Board follows.
- **Added (`rusty_tick`):** `RUSTY_TICK_ISSUER` turns on `rusty_oauth_server`'s endpoints, with grants in `<data-dir>/oauth.json`. The consent page takes one of the server's own tokens, and the grant's subject is that token's user key. Each request resolves the subject to a live user, so disabling a user in `users.json` ends their grants. OAuth access tokens are accepted wherever a token is, including inside MCP tool calls. With OAuth on, a `/mcp` 401 carries RFC 9728's `resource_metadata` challenge.
- **Added (`rusty_serve`):** `Server::with_handler_paths`, so a `GET` of `/mcp` or an OAuth path reaches the handler instead of the web UI's app shell.
- **Changed (`rusty_oauth_server`):** the consent and expiry pages are sent with a `Content-Security-Policy` (`frame-ancestors 'none'`), `X-Frame-Options: DENY` and `Referrer-Policy: no-referrer`, so the consent form cannot be framed. remind-me gets the same headers.
- **Fixed (`rusty_oauth_server`):** RFC 7009 `/revoke` dropped every token of the presented token's client. That was harmless with one owner, but several tick users can authorize one public client registration, so Bob could have revoked Alice's grants with his own token. It now drops only the presented token's grant family, meaning that client's tokens for the same subject; subjectless legacy grants are their own family. `Provider::revoke_tokens_for_client` and `StateStore::delete_tokens_for_client` are replaced by `revoke_grant` and `delete_grant`. The admin `revoke_client` still drops a whole client.
- **Known limitations:** grants outlive the token that approved them; revoking one token or changing `RUSTY_TICK_TOKEN` does not end them. Disable the user, or delete `oauth.json` with the server stopped. Interop with a live claude.ai connector is still not testable from CI.
