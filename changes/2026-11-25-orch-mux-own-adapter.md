---
category: Added
changelog: rusty_orch: `orch_bbp::mux`, the `rusty-mux-ctl own` adapter that launches #600 builder blocks with the slot at fd 3 and the ACP end at fd 4 (#1318 S4).
---
## 2026-11-25 - rusty_orch: mux `own` adapter for builders (#1318 S4)

- **Added:** `orch_bbp::mux::Ctl::window`: the project's session and window by name, made if missing (`list`, `create-session --name`, `create-window`), as #1571 §2 describes.
- **Added:** `orch_bbp::mux::Own`: one fresh `rusty-mux-ctl own` per builder, spawned with the slot and ACP end at fds 3 and 4; `launch` (C75 exact env and `fds`, C78 `hold`, C77 `owner-only`, D336 `agent`/`role`/`card`), `signal`, `release`, `close`, and owner events.
- **Added:** `viewer`, the held, display-only `tail -F acp.log` block beside the builder (D104 item 4, D187), and `exact_env` (`PATH`, `HOME`, `LANG`, `TERM` only).
- **Failure:** `lost`, `own` ending or no answer within 30 s is `MuxError::Lost` (the session ends `aborted`); a refusal is `MuxError::Refused`.
- **Dependency:** orch-bbp uses `platform` and `platform-linux` (Linux only) for `spawn_passing` (S1). No crate edge to the mux.
