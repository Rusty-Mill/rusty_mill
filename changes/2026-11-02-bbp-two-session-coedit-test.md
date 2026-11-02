---
category: Added
changelog: rusty_bbp_host: an end-to-end test where two `bbp mcp` sessions edit one co-edited spec and the slower edit is refused `StaleBase`.
---
## 2026-11-02 - Two sessions editing one spec

- **Added:** `two_sessions_edit_one_spec_and_the_slower_edit_is_refused_stale_base`
  in `rusty_bbp_host/tests/mcp_e2e.rs`. Two planners hold live turns at once,
  each through its own `bbp mcp` session process. Both read the spec at one
  version and edit the same line: the first edit lands, the slower one is
  refused `StaleBase` and stores nothing, and it lands once redone on the head.
  This closes BBP step 4's last item; the editors are scripted, no API key.
- **Known limitations:** test only. The `put_artifact` tool text says
  `stale_base` while the wire code is `StaleBase`.
