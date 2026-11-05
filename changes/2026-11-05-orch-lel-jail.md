---
category: Added
changelog: orch gains `lel-jail`, the launcher a mux agent block runs, with its `jail.json` spec.
---
## 2026-11-05 - orch: `lel-jail`

- **Added:** `orch-jail` (Let Em Loose D82, card #732, ADR-0022).
  - `jail.json` is the spec orch writes for one agent session: cgroup, status file, cwd, read and write roots, limits, the exact environment, a token by reference, and the program. Unknown fields, relative paths or `..` in any path, `RUSTY_MUX_*` variables and a token file the agent could reach are all refused.
  - `lel-jail JAIL.json` requires the slot lock on fd 3 and ACP on fd 4, joins the cgroup, and closes every other descriptor. Before the agent starts it pins every root, the cwd and the token as `O_PATH` descriptors and gives the helper those (`/proc/self/fd/N`), so a root renamed or swapped for a symlink after the check can't redirect a Landlock rule. It refuses the launch if the pinned token or its directory is inside a pinned root, a path can't be opened, any part of one could be replaced by another user, or the token has a second link. It then execs the agent under `rusty_sandbox` with `Sockets::InternetOnly`. A setup failure exits 125 before anything runs.
- **Changed:** `rusty_sandbox`'s helper sets the open-file limit last, after Landlock and seccomp, so descriptors held only for setup (close-on-exec, gone at the exec) don't count against the program's budget. The other limits are still set first.
- **Known limitations:** Linux only: the spec names Linux paths and is judged by the host's path rules, so `orch-jail` builds the spec and launcher on Linux only, and elsewhere `lel-jail` exits 125. Nothing launches it yet; the builder launch through the mux comes later. v0 has no namespaces, so egress isn't limited to the Claude endpoint and the mux socket is kept out by seccomp and Landlock, not by a mount namespace (D85). The end-to-end test skips without a writable cgroup2 directory.
