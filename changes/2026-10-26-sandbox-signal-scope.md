---
category: Security
changelog: Sandboxed jobs and confined BBP harnesses can no longer signal processes outside their sandbox on Linux 6.12 and later.
---
## 2026-10-26 - Sandboxed jobs cannot signal outside processes

- **Security:** a confined BBP harness or runner workload ran as the moderator's user and could `kill(2)` any of that user's processes: the moderator, the per-turn MCP broker, other roles' harnesses. The `Sandbox` port gains `confine_filesystem_and_signals`; the Linux backend adds `LANDLOCK_SCOPE_SIGNAL` (ABI v6) to the filesystem ruleset, restricting signals to the job's own Landlock domain, and the `rusty_sandbox` helper applies it to every job. A job can still signal its own children, and signals sent to it are unaffected. `tests/confined_harness.rs` probes it from a real confined harness.
- **Known limitations:** below Landlock ABI v6 (Linux before 6.12) the scope is not enforced and the job runs as before; `rusty_sandbox::signals_scoped()` reports which. A seccomp fallback was considered and dropped: it cannot tell a job's own child from any other process of the user, so it would break jobs that signal their children.
