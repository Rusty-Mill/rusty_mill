# rusty_rsi: status and follow-ons

As of 2026-10-05 · follows [ADR-0005](../../adr/0005-rsi-harness.md)

## Status

All four planned phases of ADR-0005 are merged, plus four follow-ups; nothing is open. `rsi` has never run against a real model: CI uses scripted models and fake agents only.

| PR | Merged | What it shipped |
| --- | --- | --- |
| [#509](https://github.com/Rusty-Mill/rusty_mill/pull/509) | 2026-10-05 | ADR-0005 §8 fixed: configuration is flags plus `RSI_*` env vars, no `rsi.toml`; lineage records model ids only (docs only) |
| [#507](https://github.com/Rusty-Mill/rusty_mill/pull/507) | 2026-10-05 | Outer-agent cost (tokens plus wall time) recorded on each proposal's lineage entry and totalled by `rsi report`; recorded, not budgeted |
| [#504](https://github.com/Rusty-Mill/rusty_mill/pull/504) | 2026-10-05 | Codex as the inner model (`RSI_INNER_PROVIDER=codex`), Claude Code as the outer proposer (`RSI_OUTER_PROVIDER=claude`) |
| [#485](https://github.com/Rusty-Mill/rusty_mill/pull/485) | 2026-10-05 | Codex CLI as the outer proposer, inside `rsi`'s Landlock sandbox |
| [#483](https://github.com/Rusty-Mill/rusty_mill/pull/483) | 2026-10-04 | P4: outer loop, accept gate, `rsi calibrate`, `rsi run`, `rsi report`, replay |
| [#478](https://github.com/Rusty-Mill/rusty_mill/pull/478) | 2026-10-04 | P3: metered broker, a0 inner agent, OpenAI-compatible and scripted models |
| [#476](https://github.com/Rusty-Mill/rusty_mill/pull/476) | earlier | P2: task format, toy tasks, sandboxed executor and graders |
| [#472](https://github.com/Rusty-Mill/rusty_mill/pull/472) | earlier | P1: `rsi-core` types, accept gate, budget meter, lineage hash chain |

`rsi` runs on Linux only. Its sandbox (Landlock, seccomp, rlimits) refuses to run anywhere else, by design.

## Next step: real-agent smoke run in WSL2

The next step is one short real run, done by the owner in WSL2 on Windows. It can't run in the cloud session: Codex isn't installed there, and its `claude` uses the session's own credentials, not the owner's subscription.

1. Open a WSL2 distro (`wsl --install -d Ubuntu`, then `wsl --update` for the newest kernel).
2. Check that the kernel has Landlock: `cat /sys/kernel/security/lsm` must list `landlock`. If it doesn't, `rsi` refuses to run; use a custom WSL kernel or a Linux VM.
3. Inside WSL, install Rust 1.98.1, `git`, `python3`, and the Linux builds of `codex` and `claude`. Log in to each from inside WSL; Windows `.exe` files and Windows logins won't work in the sandbox.
4. Clone the repo into the Linux filesystem (`~/rusty_mill`, not `/mnt/c/...`).
5. Point `rsi` at the native binaries and run the real-agent tests. Expect three passes: a Codex proposal, a Claude proposal and a metered Codex completion.

```sh
codex login            # and run /login once inside claude
export RSI_OUTER_CODEX="$(readlink -f "$(which codex)")"
export RSI_OUTER_CLAUDE="$(readlink -f "$(which claude)")"
cargo test -p rsi-cli --test agents -- --ignored
```

6. Do a two-step run with Claude as the outer agent and Codex as the inner model, so no Ollama is needed. The report should show non-zero inner and outer costs, two proposals, and a successful replay.

```sh
cargo build -p rsi-cli
export RSI_OUTER_PROVIDER=claude RSI_INNER_PROVIDER=codex RSI_INNER_CODEX="$RSI_OUTER_CODEX"
TASKS=crates/apps/rusty_rsi/crates/rsi-runtime/tasks
./target/debug/rsi calibrate --tasks $TASKS --tokens 50000 --wall-secs 600 --out cal.json
./target/debug/rsi run --tasks $TASKS --run-dir runs/smoke --steps 2 \
  --tokens 50000 --wall-secs 600 --calibration cal.json
./target/debug/rsi report --run-dir runs/smoke --replay --tasks $TASKS
```

The run makes one Codex call per inner-model request, so check the Codex plan's quota before step 6. Any failure output goes into a small fix PR.

## Open follow-on options

Recommended order: the smoke run first, since real costs and failures should shape the rest. Then budget the outer cost if the run shows the outer agent dominating spend.

| Option | What it is | Size | Needs sign-off |
| --- | --- | --- | --- |
| Real-agent smoke run | The WSL2 steps above, run by the owner | Owner time; small fix PRs if it fails | No |
| Budget the outer cost | Count the outer agent's tokens and time against a run-level budget; today they are recorded but only inner runs are budgeted | 1 PR | Yes: changes what the budget invariant covers |
| Shared git crate in `libs/` | Merge `rsi-runtime`'s git adapter with `sessionmgr-git` into one crate (ADR-0005 open question 3) | 1–2 PRs, touches another app | Yes |
| Offline inner model via `rusty_llama` | Serve the inner model in-process through `rusty_llama`'s OpenAI-compatible server; ADR says config only, no adapter code | 1 docs-and-test PR | No |
| Native Windows support | A Windows sandbox with the same guarantees as Linux; see the next section | 3–4 PRs, starting with a probe | Yes: ADR-0005 lists non-Linux executors as out of scope |

## Native Windows support

Native Windows is feasible but is a 3–4 PR project, not a port. It only counts if the Windows sandbox gives the same guarantees: private task data stays unreadable (invariant 1) and resource limits hold (invariant 5). WSL2 is the recommended route until there is a long-term need for native Windows.

### What exists and what is missing

| Need | Linux today | Windows equivalent | In the workspace? |
| --- | --- | --- | --- |
| Limits; kill the whole process tree | rlimits plus a process-group kill | Job Objects | Yes: `rusty_win32::job` |
| File access: read some paths, write only scratch | Landlock | AppContainer or LPAC, plus permission grants on allowed paths | No: rustils' `WindowsSandbox` reports itself unsupported |
| Network off for solutions, on for agents | seccomp socket rules | AppContainer capabilities (internet access for agents only) | No |
| Model calls only through the broker | Inherited Unix socketpair | Inherited anonymous pipe | Partly: `rusty_win32::pipe` |
| Blocking dangerous system calls | seccomp | None; AppContainer is the whole boundary | n/a |

### Risks

- **File permissions change.** An AppContainer reads only paths granted to it, so `rsi` must add and later remove grants on the Python, Rust and agent-CLI installs.
- **The agent CLIs may not run inside it.** Node-based tools sometimes break in an AppContainer; Codex and Claude on Windows are unproven there. The probe answers this before any design commitment.
- **Weaker system-call filtering.** Windows has no seccomp equivalent, so the ADR amendment must argue that AppContainer plus a Job Object is enough on its own.

### Plan

1. **Probe (small PR).** Show that Codex, Claude, `python3` and `rustc` run inside an AppContainer with only granted access, and that network access can be switched on and off. A failure here stops the project.
2. **Windows sandbox in rustils.** AppContainer plus Job Object, with the same private-data tests running on CI's `windows-latest`.
3. **Wire into `rsi`.** Executor, broker pipe, and permission grants for the toolchains.
4. **ADR-0005 amendment and docs.** Move non-Linux executors from out of scope to supported, with the equivalence argument.

## Decisions needed

- [ ] Run the WSL2 smoke run, and confirm the WSL kernel lists `landlock`.
- [ ] Should the outer agent's cost count against a run-level budget, or stay recorded only?
- [ ] Approve the shared git crate in `libs/`, or leave it for later.
- [ ] Native Windows: start the probe now, or stay on WSL2 until there is a concrete need?
- [ ] Optional: run the smoke test in the cloud session instead. That needs Codex installed there and the owner's Codex login in the container, so only with the owner's explicit OK.
