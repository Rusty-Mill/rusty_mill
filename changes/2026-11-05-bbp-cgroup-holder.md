---
category: Added
changelog: bbp gains a cgroup holder (`slot-hold --cgroup`), `cgroup-open`, and cgroup-aware `quiesce` for mux-hosted agent sessions.
---
## 2026-11-05 - bbp: cgroup holder for mux-hosted agents

- **Added:** containment by cgroup for agents hosted in mux blocks (Let Em Loose D82, D86, card #732).
  - `bbp cgroup-open --cgroup PATH` records the session's cgroup under the task, then creates it.
  - `bbp slot-hold --cgroup PATH [--deadline-ms N]` keeps the slot until stdin closes and the cgroup is empty. At the deadline it writes `cgroup.kill`. It never signals a process group.
  - `bbp quiesce` also kills populated recorded cgroups and removes empty ones with their records.
- **Known limitations:** nothing uses these yet; the builder launch (slice 6) takes the slot through the holder and hands its fd to orch. The end-to-end tests skip without a writable cgroup2 directory.
