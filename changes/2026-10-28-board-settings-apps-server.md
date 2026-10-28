---
category: Added
changelog: Mill Board settings: an Apps tab lists and revokes OAuth sign-ins, a Server tab shows how the board was started, and Roles explain each capability.
---
## 2026-10-28 - Mill Board: manage signed-in apps from settings

- **Added:** an **Apps** tab in Mill Board's settings lists the apps signed in through OAuth (name, client id, when registered, live tokens) and revokes one: its registration and tokens are dropped, and it can sign in again. Backed by a new admin-only `settings_app_revoke`; `settings_get` returns `apps` (`null` when `--issuer` is off). Before, signing out one connector meant removing its agent or editing `oauth.json` with the board stopped.
- **Added:** a read-only **Server** tab: where the cards live (rusty_tick URL or memory), the OAuth issuer, the accepted hosts and the path of each settings file.
- **Added:** the Roles tab describes each capability, and its columns come from the server's capability list instead of a copy in the page, so a new capability appears without a UI change.
- **Known limitations:** revoking signs out every user of that app registration; per-user grants are not listed. Server settings still change only with the service's flags or `board.env` and a restart.
