---
category: Added
changelog: rusty_orch: `rusty_orch builder run` and `orch_bbp::builder`, one #600 builder session on one claim, end to end (#1318 S5).
---
## 2026-11-25 - rusty_orch: `builder run` (#1318 S5)

- **Added:** `orch_bbp::builder::run`, the launch sequence over one `Ports` trait: prepare, hold, open, one ACP turn under the wall clock, then `term`, 60 s grace, orch's `cgroup.kill`, `release` (D104 §2 step 7).
- **Added:** `rusty_orch builder run <config> --bbp --dir --task --claim --state --repo --base --cgroup-parent --slots --mux-ctl [--mux-socket] [--jail] [--read]...` (Linux), printing the session row as one JSON line (D189 item 1).
- **Added:** `lens::bbp::Session::put_candidate`: a diff then a candidate, stable op ids, one retry on a lost answer.
- **Rule:** a candidate is posted only after `end_turn` with `.lel/submit` and a non-empty diff (D188); a lost mux ends `aborted`; the holder's deadline is wall + grace + 30 s (D189 item 2).
- **Dependency:** orch-bbp uses orch-lens; rusty_orch uses orch-jail, `platform`, `platform-linux` (Linux only). All in-workspace.
- **Security:** the diff is taken with orch's own worktree git dir, read before the agent runs, with hooks, fsmonitor, external diff and textconv off, so a `.git` the agent rewrites runs nothing outside the jail. A worktree holding any nested `.git` (a repo at a submodule path would have `git add` read its config) gets no diff.
- **Added:** `orch_bbp::bounded::output_within`: git, `bbp` and `rusty-mux-ctl` calls are killed past a bound, so no step can hold a session at `admitted`.
