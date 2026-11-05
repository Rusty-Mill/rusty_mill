---
category: Fixed
changelog: rusty_tick `user adopt` leaves `oauth.json` in the data directory, so connectors stay signed in when a store switches to per-user tokens; the deploy guide shows the switch.
---
## 2026-11-05 - rusty_tick: keep OAuth grants when adopting a single-user store

- **Fixed:** `rusty_tick user adopt` moved `oauth.json` into the user's store along with the tasks, where the server never reads it, so every claude.ai connector had to sign in again. It now stays in the data directory. Adopting as `default` (the subject single-user grants carry) keeps existing grants valid.
- **Added:** the deploy guide's "From one shared token to a token per client" section: adopt, mint a token per client, drop `RUSTY_TICK_TOKEN`, roll back.
- **Known limitations:** the old shared token stops working at the switch; every client needs its new token.
