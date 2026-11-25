---
category: Added
changelog: platform-linux 0.30.0 and rusty_libc: `LinuxSpawner::spawn_passing` puts passed descriptors at 3, 4, … for a plain spawn, and `LinuxChild::dup_fd` copies a child's descriptor through `pidfd_getfd` (#1318 S1).
---
## 2026-11-25 - platform-linux, rusty_libc: passed descriptors and pidfd_getfd (#1318 S1)

- **Added:** `rusty_libc::process::pidfd_getfd`, beside `pidfd_open` and `pidfd_send_signal`.
- **Added:** `platform_linux::LinuxSpawner::spawn_passing(cmd, passed)`: the child gets `passed` in order at 3, 4, … and nothing else above 2, the same rule as `LinuxPty::spawn_passing` (C75). The staging and mapping code moved from `sys::pty` to `sys::spawn` and serves both.
- **Added:** `platform_linux::LinuxChild::dup_fd(fd)`: a close-on-exec copy of the child's descriptor that shares its open file description and `flock` locks; refused once the child is reaped.
- **Changed:** the PAL group is 0.30.0 (a public-API addition, `docs/versioning.md` §2). `sys::spawn::spawn` takes a trailing `passed` slice; `Spawner::spawn` is unchanged.
- **Why:** rusty_orch's builder launch (#1318, D188 design) takes the cgroup holder's slot descriptor with `dup_fd` and starts `rusty-mux-ctl own` with the slot at fd 3 and the ACP end at fd 4, with no `unsafe` of its own.
