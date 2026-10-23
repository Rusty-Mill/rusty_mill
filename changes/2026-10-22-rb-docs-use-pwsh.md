---
category: Changed
changelog: rb_tape_bot docs and the run_batch.ps1 header tell people to run it with pwsh, and a mangled README example is repaired.
---
## 2026-10-22 - rb_tape_bot: docs run the batch with pwsh

- **Changed:** the `rb_tape_bot` README, the `run_batch.ps1` header and the BOT-RUN-SHEET command now say
  `pwsh -NoProfile -File tools\rb_tape_bot\run_batch.ps1` instead of `powershell -File`. The repo's tooling targets
  pwsh (the README's own spread example already used `Join-String`, which Windows PowerShell 5.1 lacks).
- **Fixed:** the README's 50-minute spread example had its path split across three lines at the backslash-r
  sequences of `tools\rb_tape_bot\run_batch.ps1`, so it could not be pasted; it is one line again.
- **Known limitations:** `rb_rl_game_bridge/src/ports.rs` still runs `powershell` itself; that is behaviour, not
  documentation, and is not changed here.
