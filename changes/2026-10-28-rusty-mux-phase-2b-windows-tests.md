---
category: Changed
changelog: rusty_mux's wait tests and a terminal.process test now run on Windows, closing out Phase 2b's exit evidence; the mux docs cite the right PRs.
---
## 2026-10-28 - rusty_mux Phase 2b tests on Windows

- **Changed:** `rusty-mux-ctl wait --block`, its timeout and closed-block exits, and `wait --event` are tested on CI's Windows leg as well as Linux (`ctl/tests/ctl.rs`).
- **Changed:** a Windows server test checks that `terminal.process` lists the block's program by pid (`server/tests/windows.rs`).
- **Changed:** ROADMAP and CAPABILITIES cite #1183, #1189, #1190 and #1200, and record that the #889 throughput floors are met (#1019).
- **Known limitations:** bell, progress, notification, DECCKM cursor keys and `terminal.format --unwrap --vt` stay Linux-only tests: they depend on ConPTY pass-through (R1).
