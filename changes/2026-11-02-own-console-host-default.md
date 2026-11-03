---
category: Changed
changelog: On Windows, rusty_term, rusty_mux_server and rusty_yirp now run shells on our own console host (rusty_conhost) by default; `conpty` stays selectable (#1222).
---
## 2026-11-02 - Our own console host is the Windows default

- **Changed:** on Windows, rusty_term's `[console] host`, and
  `RUSTY_CONSOLE_HOST` for rusty_mux_server and rusty_yirp's session
  daemon, now default to `own` (rusty_conhost) instead of `conpty`. This is
  Phase B of the console host design (`docs/projects/term/conhost/`,
  §9.2): the parity suite passes with no gaps, the throughput gate passes,
  and the manual gate passed on two machines.
- **Changed (API):** `ConsoleHost::default()` is now `Own` in `rusty_term`,
  `rusty_mux_server` and `sessionmgr-pty`, so code that builds a config
  with `ConsoleHost::default()` gets our host on Windows. Other platforms
  ignore the host.
- **Rollback:** set `[console] host = "conpty"` or
  `RUSTY_CONSOLE_HOST=conpty`. Explicit values behave as before.
- **Known limitations:** if this Windows has no usable ConDrv, the default
  now fails with an error naming the cause instead of starting on ConPTY;
  there is still no silent fallback. The web bridge stays on ConPTY.
