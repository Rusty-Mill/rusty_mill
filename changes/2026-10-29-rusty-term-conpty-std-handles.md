---
category: Fixed
changelog: rusty_term's ConPTY child no longer writes to the parent's redirected std handles, and its two ConPTY attach tests run again.
---
## 2026-10-29 - rusty_term ConPTY spawn sets STARTF_USESTDHANDLES

- **Fixed:** rusty_term's Windows backend now sets `STARTF_USESTDHANDLES`
  when it spawns the shell under ConPTY. Without it, a parent whose own std
  handles were redirected (any `cargo test` or CI run) had them duplicated
  into the child, so the child's output bypassed the pseudo console.
  platform-windows already carried the same fix.
- **Fixed:** the two ConPTY attach tests, ignored since July as an Insider
  build regression, run again. Both fail without the fix and pass with it on
  Windows 11 build 26300.
