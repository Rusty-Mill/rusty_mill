---
category: Added
changelog: rusty_bbp_host: `bbp slot-hold --cgroup PATH --slots DIR` takes its own agent slot and prints the descriptor it holds it on (#1318 S2, D108 item 2).
---
## 2026-11-25 - rusty_bbp_host: the cgroup holder takes its own slot (#1318 S2)

- **Added:** `bbp slot-hold --cgroup PATH --slots DIR [--deadline-ms N]`. The holder takes a free agent slot from `DIR` with the capacity already recorded there, prints the descriptor number as stdout's first line, then holds as before: released only once stdin is closed and the cgroup is empty, `cgroup.kill` from the deadline. With every agent slot held it exits non-zero and takes nothing. `--slots` without `--cgroup` is refused.
- **Added:** `cgroups::hold_slot`, `slots::Slots::recorded` and `slots::Slot::fd`.
- **Unchanged:** plain `slot-hold` and `slot-hold --cgroup` without `--slots` still hold the slot they are given on stdout.
- **Why:** rusty_orch's builder launch (#1318) starts the holder, copies that descriptor with `pidfd_getfd` (`LinuxChild::dup_fd`, S1) and passes it to the jail as fd 3, so the holder, orch and the jail share one lock (D108 item 2). orch-bbp stays CLI-only and never links `rusty_bbp_host`.
