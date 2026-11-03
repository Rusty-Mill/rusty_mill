---
category: Changed
changelog: rusty_mux roadmap: C71 (`terminal.modes`, `terminal.clear`, `terminal.shell`) marked done.
---
## 2026-11-03 - rusty_mux C71 done

- **Changed:** CAPABILITIES marks C71 done (#1440, #1441), ROADMAP step 8 of Phase 3b records both PRs, and their DECISIONS rows record the acceptance given on each PR.
- **Known limitations:** `terminal.clear` is refused while the program's output stops inside an escape sequence. `terminal.shell` reports `activity` as `unknown` on Windows and judges `busy` by the foreground process group; its OSC 133 lines stay `unknown` for a shell without that integration, and that state is not checkpointed. Neither is validated with live clients or on Windows.
