---
category: Added
changelog: orch gains `lel-jail`, the launcher a mux agent block runs, with its `jail.json` spec.
---
## 2026-11-05 - orch: `lel-jail`

- **Added:** `orch-jail` (Let Em Loose D82, card #732, ADR-0022).
  - `jail.json` is the spec orch writes for one agent session: cgroup, status file, cwd, read and write roots, limits, the exact environment, a token by reference, and the program. Unknown fields, relative paths, `RUSTY_MUX_*` variables and a token file the agent could reach are all refused.
  - `lel-jail JAIL.json` requires the slot lock on fd 3 and ACP on fd 4, joins the cgroup, closes every other descriptor, then execs the agent under `rusty_sandbox` with `Sockets::InternetOnly`. A setup failure exits 125 before anything runs.
- **Known limitations:** nothing launches it yet; the builder launch through the mux comes later. v0 has no namespaces, so egress isn't limited to the Claude endpoint and the mux socket is kept out by seccomp and Landlock, not by a mount namespace (D85). The end-to-end test skips without a writable cgroup2 directory.
