---
category: Added
changelog: rusty_mux shows an agent's status from its hooks over the program's OSC 7501 status (`terminal.report-status`, `agent hook --event`), starting with OpenCode and Pi.
---
## 2026-11-04 - rusty_mux agent status from hooks (C74, O2 part 1)

- **Added:** block method `terminal.report-status` behind `Features::AGENT_STATUS` (bit 33). A
  hook's report is shown over the program's own OSC 7501 report until the hooks have been silent
  for 120 s and the program reports something else.
- **Added:** `rusty-mux-ctl agent hook --event STATE [--id] [--kind] [--app] [--msg]`, which sends
  it for the block it runs in, silently, always exiting 0.
- **Changed:** the OpenCode plugin and Pi extension that `agent setup` installs now report turns,
  tools and permission waits. Installed copies show `outdated` until `agent setup` runs again.
