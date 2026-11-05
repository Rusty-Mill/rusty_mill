---
category: Added
changelog: bbp gains a cgroup holder (`slot-hold --cgroup`), `cgroup-open`, and cgroup-aware `quiesce` for mux-hosted agent sessions.
---
## 2026-11-05 - bbp: cgroup holder for mux-hosted agents

- **Added:** containment by cgroup for agents hosted in mux blocks (Let Em Loose D82, D86, card #732).
  - `bbp cgroup-open --cgroup PATH` claims the path for one task, records it under the task, then creates the cgroup. Each step is exclusive and runs under a per-task cgroup lock, and an ended task is refused.
  - `bbp slot-hold --cgroup PATH [--deadline-ms N]` keeps the slot until stdin closes and the cgroup is known to be empty. From the deadline on it writes `cgroup.kill`. A failed read or kill is reported and retried, and never frees the slot. It never signals a process group.
  - `bbp quiesce` also kills populated recorded cgroups and removes empty ones with their claims and records. It does this under the same lock, and only for paths its task owns.
- **Known limitations:** nothing uses these yet; the builder launch (slice 6) takes the slot through the holder and hands its fd to orch. The end-to-end tests skip without a writable cgroup2 directory.
