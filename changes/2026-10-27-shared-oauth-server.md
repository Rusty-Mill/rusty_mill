---
category: Changed
changelog: "`rusty_oauth_server`: new framework-free OAuth 2.1 authorization server crate, extracted from `remind_me_remote` so rusty_tick and Mill Board can offer claude.ai \"Sign in now\" too; remind-me now runs on it with its state file unchanged."
---
## 2026-10-27 - Shared OAuth server crate, extracted from remind-me

- **Why:** rusty_tick and Mill Board need the same OAuth flow remind-me has. They serve HTTP on `rusty_serve`, remind-me on `rusty_axum`, so copying remind-me's axum module would not fit them. This is the first of three PRs; tick and the board follow.
- **Added (`rusty_oauth_server`, libs/net):** issuer validation, the JSON state store, client and token types, the provider (consent, PKCE code grant, refresh rotation, revocation) and the endpoints, all as plain request-in, response-out code with no HTTP framework. Consent is approved by pasting a token the server already accepts; a `Verify` function turns it into a `Subject`, every grant records that subject, and `Provider::subject_of` returns it for an access token, so a server can map OAuth grants onto its own users. JSON is `rusty_json`, so a caller avoiding `serde_json` does not gain it.
- **Changed (`remind_me_remote`, `remind_me_core`):** the OAuth module is now an axum adapter over the crate, about 150 lines in place of about 2,400. `remind_me_core::remote::OAuthStateStore` and its companion types are re-exports under their old names, so `remind_me_revoke_clients` and `remote_status` are unchanged. Existing state files load as they are (a test pins remind-me's layout and token hashes), and their grants keep working with no subject.
- **Behavior differences:** a PKCE `code_verifier` must now be RFC 7636-valid (43 to 128 unreserved characters), because verification uses `rusty_oauth`'s RFC check. A form or JSON body missing a required field is a `400` OAuth error instead of the framework's rejection. Generated ids and tokens are 64 hex characters from the OS random source instead of two UUIDs.
- **Known limitations:** interop with a live claude.ai connector is still not testable from CI; the HTTP-level tests drive the real server.
