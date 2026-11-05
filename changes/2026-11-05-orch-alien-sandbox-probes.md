---
category: Added
changelog: The alien lens runs its probes itself, sandboxed with no network, on scratch worktrees of the candidate and base (#708).
---
## 2026-11-05 - Alien lens sandboxed probe runner

- **Added:** `rusty_orch::lens::probes::SandboxProbes` is the shell's `Probes` (D89). For each probe it makes a scratch worktree of the candidate's base with each diff applied, writes the probe to `tests/<file>` in the crate its site names, and runs `cargo test --offline -p <crate> --test <probe>`: the build with `--no-run` first, then the test. Only when the candidate fails does it do the same on a base tree. Every cargo run is confined by `rusty_sandbox` with no socket endpoints, and each has a 10-minute wall limit (`PROBE_CARGO_LIMIT`, D90). Trees live under `<state>/alien/probes/<task>/<claim>-<n>/` and are removed when the probe is done. Each tree has its own temp dir, `HOME` and cargo home. The candidate's build cache is kept per hunt, and base builds into a fresh target dir, so a candidate run can't affect base. Writes into a tree refuse to go through a symlink, so a candidate's diffs can't redirect the probe file outside it.
- **Added:** a hang or a kill by signal on the candidate is a build error, so it is an `error` row and never a finding. On base, either one means the probe did not run, so it gets no row. A site that names no crate in the tree cannot build.
- **Added:** `rusty_orch __sandbox` is the sandbox helper that `rusty_sandbox` re-invokes. It is not for direct use.
- **Changed:** `rusty_orch` depends on the in-workspace crates `rusty_sandbox` and `rusty_worktree`.
- **Known limitations:** nothing calls the runner yet; the `--alien` runner pass wires it in. Probes build `--offline`, so their dependencies must already be in the user's cargo caches, which the sandbox can read but not write.
