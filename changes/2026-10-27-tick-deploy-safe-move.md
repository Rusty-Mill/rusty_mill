---
category: Fixed
changelog: rusty_tick's deploy guide moves a store before the first start and upgrades the binary and web UI together.
---
## 2026-10-27 - rusty_tick deploy: safe store move and whole UI swap

- **Fixed:** the guide started the service on an empty store before the move step, and an upgrade's `cp -r dist web` nests `dist` inside an existing `web/`. A move is now a complete snapshot put in place with both servers stopped and checked before clients switch, with the token rule for each mode. An upgrade swaps in the binary and web UI together and keeps both previous copies for rollback.
