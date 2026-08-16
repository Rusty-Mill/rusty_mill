# Windows AI Coding-Agent Session Manager — Implementation Plan

## Context

This follows a research thread that started with inspecting `Xirp-0.14.0-x64-external.dmg` (Spotify's closed-beta, macOS-only Electron app for managing Claude Code / Codex / Gemini CLI sessions in isolated git worktrees). That inspection stayed clean-room throughout — no Xirp source/assets were ever extracted or read; `CAPABILITIES.md` was built entirely from five public sources (review videos, Spotify's own announcement, third-party analysis), each capability tagged by which source confirmed it and how reliable that source is. Comparable Windows-available tools were researched directly (Solo: multi-agent process management but explicitly no worktree isolation; Conductor: worktree isolation but Apple-Silicon-only) — the user confirmed the target is Xirp's full feature set, not a narrower gap-filling scope, and confirmed four fixed constraints: `rusty_tokio` as the async runtime, all three agent CLIs (Claude Code/Codex/Gemini) from day one, a TUI dashboard in v1, and the full CAPABILITIES.md feature set as the real target rather than a cut-down MVP.

Before designing anything, two Explore passes verified every crate SCOPE.md assumed was available/reusable, by reading the actual code rather than trusting an earlier design conversation's memory of it. This surfaced several corrections that reshape the architecture (detailed below) — most importantly, that Job Objects (Windows' kill-on-close process-group primitive) are structurally incompatible with "sessions persist independently of the manager app closing," a capability directly confirmed in CAPABILITIES.md (source 3: Xirp sessions keep running under tmux after quitting the app). The corrected design instead follows a real, working precedent already on this machine: `rusty_prime_agent`'s (`C:\dev\rusty_prime_agent`) detached-worker-process + crash-recovery daemon architecture, which independently solved this exact problem and documented the Job Object conflict in its own `ARCHITECTURE.md`.

This plan was then adversarially reviewed by a second pass instructed to find real problems, not rubber-stamp the design — its findings (13 issues, 2 blocking) are incorporated below, with one of its own claims corrected against direct code re-inspection. The goal of writing this out explicitly is that the challenge-and-fix loop is visible in the plan itself, not just something that happened invisibly during planning.

**Outcome this plan is working toward**: a `sessionmgr` Rust workspace producing a single Windows binary that manages Claude Code / Codex / Gemini CLI sessions, each optionally isolated in its own git worktree, presented through a TUI grid dashboard, with sessions that survive the manager app closing — phased so the highest-uncertainty parts (agent-CLI "needs input" detection, `rusty_tokio`'s real availability) are proven earliest.

---

## Corrected facts superseding SCOPE.md (verified by direct code inspection, not assumed)

- **`rustils`' Windows "Job Object" code is a kill-on-close process-group lifecycle primitive, not a security sandbox.** `platform::security::Sandbox`'s Windows implementation is a real trait with zero implementation — every method returns `SandboxStatus::Unsupported`. Nothing in this project should ever be described as "sandboxed" on Windows.
- **Job Objects conflict with session persistence.** `rusty_prime_agent` discovered and documented this exact tension in its own `ARCHITECTURE.md` ("a kill-on-close Job Object would defeat detach's own 'survives a crash' guarantee") and deliberately does not use rustils' Job Object mechanism for its worker processes — using instead a hand-rolled `procutil.rs` (`prepare_detached`: `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`; `is_alive`: `OpenProcess` + `GetExitCodeProcess`; `kill`: `TerminateProcess`) on top of `rusty_tokio::process::Command`'s `as_std_mut()` escape hatch, plus clearing `HANDLE_FLAG_INHERIT` on its own inherited stdio before spawning (`Cargo.toml`'s own dependency-comment documents this; a Windows handle-inheritance detail worth carrying into this project's own detached-spawn helper).
- **`rustils`' Job Object assignment happens entirely inside its own `Spawner::spawn` → `proc::spawn(...)` call path** (confirmed directly, `platform-windows/src/process.rs:233-267`) — a `rusty_tokio`-spawned child never produces a rustils `Child`/job handle. There *is* a retrofit path — `Spawner::adopt(pid)` (`OpenProcess` + fresh Job Object + `AssignProcessToJobObject` against an already-running pid this crate didn't spawn) — but its own docs flag a race window (grandchildren spawned before the `adopt()` call escape the job), and using it means running two parallel process-tracking stacks (`rusty_tokio::Child` + rustils' `GroupHandle`) for the same pid. **This plan does not use Job Objects at all in the default v1 path** (see Process supervision, below) — `adopt()` is noted as a possible, explicitly-optional post-v1 hardening path only.
- **`rusty_tokio` is real and currently reachable** (`git ls-remote` succeeded during planning; `main` is ahead of `rusty_prime_agent`'s pinned rev) but is **not a locally-maintained sibling repo** — it exists on this machine only as a stale cargo git-dependency cache pulled in by an unrelated project. Its process module genuinely is built on `std::process` (not rustils), with async reactor-driven I/O on Unix but `spawn_blocking`-backed (not IOCP/overlapped) child stdio on Windows — a documented, acceptable-for-this-scale limitation, not a bug to fix here.
- **`rusty_prime_agent`'s reusable pattern**: one binary, three roles (daemon / re-exec'd detached worker / CLI client), `state.json` (pointer) + `transcript.jsonl` (source of truth) persistence split, black-box subprocess test convention (`tests/common/mod.rs`, spawns `env!("CARGO_BIN_EXE_...")` against an isolated temp dir, deliberately not unit-testing internals for daemon/worker coverage), and a `[lib]` + `[[bin]]` Cargo split specifically so internal wire types are importable by tests/other crates without depending on the bin target.
- **Dev-standards repos** (`Atlas_Engineering_Standards_Library`, `rusty_foundation_akb`) are both specification-stage with no concrete Rust style guide yet — only principles worth keeping visible in this plan: correctness over convenience, explicitness, economy (simplest design that solves the demonstrated problem, no speculative tooling), composability, security-as-architecture not a late feature.

---

## Workspace structure

Ports-and-adapters, `core` has zero I/O. One workspace, one shipped binary (`sessionmgr.exe`), plus a small protocol crate to fix the review's blocking finding that a bin-only daemon crate can't be depended on by the TUI crate.

```
sessionmgr/
  Cargo.toml                      # workspace, resolver = "2"
  crates/
    sessionmgr-core/              # pure domain: Session state machine, worktree/session-kind policy, ports (traits)
    sessionmgr-protocol/          # wire types only: Request/Response/SessionEvent, serde, no I/O — shared by daemon + tui
    sessionmgr-git/               # adapter: shells out to `git worktree`/`git diff`/`git status`
    sessionmgr-proc/              # adapter: detached spawn, liveness/kill (rusty_prime_agent's procutil pattern), PTY wrapper
    sessionmgr-agents/            # adapter: AgentAdapter trait + claude_code.rs/codex.rs/gemini_cli.rs + pattern_watch.rs
                                   #   sessionmgr-mcp's config parsing folded in here (no separate crate yet — see Adversarial findings)
    sessionmgr-daemon/             # [[bin]] "sessionmgr" + [lib]: supervisor, worker, catalog — the composition root
      src/
        main.rs                    # subcommand dispatch: daemon | __worker-main | __hook-fire | tui | new|list|attach|close
        supervisor.rs
        worker.rs
        catalog.rs
        hooks/                     # module, not a crate, until Phase 4 proves it needs to be one (install.rs, dispatch.rs)
        paths.rs
    sessionmgr-tui/                # ratatui + crossterm; depends on sessionmgr-protocol only, never sessionmgr-proc/agents directly
      src/
        app.rs, grid.rs
        panes/{session_pane.rs, git_diff_pane.rs, terminal_pane.rs}
        client.rs                  # daemon-socket client
  tests/
    common/mod.rs                  # black-box harness, CARGO_BIN_EXE_sessionmgr, rusty_prime_agent-pattern TempDir
    session_lifecycle.rs
    worktree_lifecycle.rs
    supervisor_restart_recovery.rs # the single most important test — proves the persistence design actually works
    worker_crash_recovery.rs
    agent_needs_input_*.rs         # gated on real installed CLIs, skip cleanly if absent
```

Deliberately **not** separate crates yet (adversarial finding #10 — economy over premature boundaries): MCP config plumbing lives inside `sessionmgr-agents` until it has real independent surface area; session hooks live as a `sessionmgr-daemon::hooks` module until Phase 4 actually needs the split.

---

## Process supervision & session persistence

**This overrides SCOPE.md's "Job-Object-based sandboxing on every spawned child" entirely** — kill-on-close is incompatible with the persistence requirement, per the corrected facts above.

**One binary, three roles**, following `rusty_prime_agent::worker::spawn`'s precedent directly:
- `sessionmgr daemon` — long-running supervisor, owns the session registry and a public local socket, is the process meant to outlive the UI.
- `sessionmgr __worker-main --session-id <id> --state-root <path>` — hidden subcommand. The daemon re-execs its own binary (`std::env::current_exe()`, always correct by construction) with a detached-spawn helper mirroring `procutil::prepare_detached` (Windows: `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` via `rusty_tokio::process::Command::as_std_mut().creation_flags()`, plus clearing `HANDLE_FLAG_INHERIT` on inherited stdio first). One worker per session, owning the PTY/piped stdio to the actual agent-CLI child, a `transcript.jsonl` writer, a `state.json` pointer file, and a private per-session socket fanning live output to attached UI clients via a broadcast channel.
- `sessionmgr tui` / `new`/`list`/`attach`/`close` — client roles; auto-start a daemon transparently if none is running (mirrors `rusty_prime_agent`'s own sugar), which is what makes `sessionmgr new` useful standalone before the TUI exists (Phase 1).

**Why one binary**: no version-skew risk between "the running daemon" and "the worker binary it just spawned," one release artifact, and `worker::spawn`'s `Command::new(exe_path)` is trivially correct. The TUI is a role of the same binary (a thin socket client), not a separate process — acceptable to link `ratatui`/`crossterm` into the daemon build even when running headless.

**Liveness tracking**: on daemon startup and periodically, scan `sessions/<id>/state.json`; for any session claiming `Running`, probe the recorded worker pid via a hand-rolled `is_alive()` (not `rusty_tokio::Child::try_wait`, which only works on an owned handle — this daemon didn't necessarily just spawn the workers it's checking, e.g. after its own restart). Alive → adopt (mark running, allow reattachment), **no respawn**. Dead → mark crashed/errored in `state.json`, do not silently resurrect (resurrecting a stale agent-CLI conversation is out of scope for v1).

**Closing a session — corrected from the design pass's false-choice framing (adversarial finding #3, refined by direct re-verification)**: no Job Object is used in the default spawn or close path at all. Graceful close sends a shutdown request over the worker's private socket first. If it doesn't ack in time: **`TerminateProcess` against both the recorded worker pid *and* the recorded agent-CLI child pid** (not just the worker — this is the fix for adversarial finding #2, which correctly identified that killing only the worker leaves the agent-CLI process orphaned with no remediation path). Then attempt `git worktree remove`; if it still fails because something still holds a file handle open in the worktree, surface *which* process (a handle-enumeration/`tasklist`-equivalent check), not just the bare failure. A Job-Object-based tree-kill via `rustils`' `adopt(pid)` retrofit is documented here as a **possible future hardening option**, not built for v1, given its race window and the two-stack complexity it would add for a case (a wedged worker's own un-tracked grandchildren) the pid-pair kill already covers in the common case.

**`SameDirectory` sessions get no collision protection** (adversarial finding #6) — documented plainly in-app and in docs as "your responsibility, same as Xirp's own model," not silently assumed safe. No `.git/index.lock` mitigation is built for v1.

**Open, unresolved-by-design tension (needs a Phase 1 spike, not assumed either way)**: real PTY (rustils' `platform-windows::pty::WindowsPty`, ConPTY-backed — likely necessary since CLIs like Claude Code change output behavior when not attached to a real terminal) vs. plain piped stdio. rustils' PTY path explicitly does not apply Job Objects or detached-spawn flags — it was built for interactive foreground use. Whether a ConPTY-attached child survives the *worker* process crashing uncleanly (not a graceful `ClosePseudoConsole`) is unverified and must be spiked in Phase 1 before committing either way.

---

## Git worktree lifecycle

Shell out to `git worktree add`/`remove` (unaffected by corrections — still the right call vs. gitoxide). Worktrees live under `<repo>/.sessionmgr-worktrees/<ulid>` (short, collision-proof ids to reduce Windows path-length exposure), branch `sessionmgr/<ulid>` by default. `close --merge` defaults to fast-forward-only, failing loudly rather than silently 3-way-merging; `close --discard` force-removes.

**Three-way session-start distinction, replicated from Xirp rather than simplified** (per the confirmed full-scope decision): `SessionKind::{SameDirectory, Worktree, PlainTerminal}`. `PlainTerminal` has zero agent-CLI uncertainty (just a PTY running the user's shell) and is the cheapest kind — built first, in the walking skeleton.

**Windows-specific mitigations** (adversarial findings #7, #8, both previously absent from the risk list): the app manifest opts into `longPathAware` (short session ids alone don't protect against a deeply-nested *target repo's* own path length); Phase 2's testing plan explicitly includes a worktree-heavy smoke test with Windows Defender at its default (not excluded) settings, since real users won't have it excluded.

---

## Per-agent-CLI adapters

**Explicitly the highest-uncertainty, highest-effort part of this project** — sequenced first (Phase 1-3), not last.

`AgentAdapter` trait: `launch_args(...)` (low-risk, mostly reading each CLI's `--help`) and `needs_input(recent_output) -> SessionSignal{Running, NeedsInput, Finished, Errored}` (the hard part), tiered by confidence:

1. **Hooks where available.** CAPABILITIES.md confirms Xirp uses Claude Code's own hook mechanism as its primary status signal, not output parsing — this plan does the same. **Unverified and gated behind a Phase-1 spike, elevated to a go/no-go gate before Phase 3 starts** (adversarial finding #5, correcting the original design's under-weighting of this risk): does Claude Code's hook mechanism fire reliably when launched as a detached, non-console-attached child from a Windows Rust process? Every public observation of Xirp using hooks was on macOS with a normally-launched process — this is genuinely unverified, not assumed.
2. **Process exit code as the unambiguous `Finished`/`Errored` signal**, all three CLIs — free from `rusty_tokio::process::Child::wait()`, always reliable, independent of hooks/parsing.
3. **Output pattern-matching as the fallback** for `NeedsInput` — the least reliable tier, standing/permanent risk (not a one-time unknown), mitigated with a per-CLI `--version` check that warns (not fails) on mismatch. **Decision required before Phase 3, not deferred**: if Codex and/or Gemini CLI turn out to have no hook mechanism at all (plausible — CAPABILITIES.md only confirms hooks for Claude Code), pattern-matching becomes the *permanent primary* signal for those CLIs, not a temporary fallback — the UI should show a visibly lower-confidence status badge for CLIs running on tier-3 detection only, rather than presenting uniform confidence across all three.

Each adapter's pattern set lives in its own file so a CLI's next release breaking its prompt format is a one-file fix, not a shared-code risk to the other two adapters.

---

## TUI design

`ratatui` + `crossterm`. Reuses rustils' existing ConPTY-backed `WindowsPty` (`platform-windows::pty`) rather than adding `portable-pty` as a second, redundant PTY dependency — the daemon's worker role is the only place that touches a PTY at all; the TUI only ever renders bytes streamed over the daemon socket, which is also the correct ports-and-adapters boundary.

**Grid/multi-pane layout**: `ratatui::layout` with configurable rows/columns, mirroring CAPABILITIES.md's "add all"/configurable-grid behavior. Pane resize is keyboard-driven (a resize mode + arrow keys adjusting `Constraint::Percentage`) — the honest TUI translation of Xirp's mouse-drag handles, not a forced mouse-drag imitation.

**Fork / switch-agent-mid-session / dependent sessions**:
- **Switch-agent-mid-session** (the standout, hardest feature) is explicitly deferred to Phase 6+, pending a per-CLI research spike into whether `--resume`/`--continue`-style flags exist and what state they actually accept.
- **Fork — resequenced per adversarial finding #4**, which correctly identified that the original design's Phase 5 placement assumed copying `sessionmgr`'s own transcript into a new session is sufficient to seed a live agent-CLI conversation, when in fact that requires the *same* per-CLI state-translation primitive switch-agent needs (sessionmgr's transcript format isn't any CLI's native resumable format). **Fork moves to Phase 6+, grouped with switch-agent under the same research spike.** If an earlier, lower-fidelity fork-like feature is wanted before that spike resolves, it should ship explicitly labeled as "restart fresh with a summarized system-prompt injection from the prior transcript," not implied to preserve full state.
- **Dependent sessions**: a `parent_id` field plus a `wait_for_parent` check the daemon evaluates before spawning the child worker (poll parent's `Finished` signal, or "start now" skips it) — real, buildable in Phase 5 without the state-translation dependency the above two features have.

**Git diff panel**: shells to `git diff`/`git status --porcelain`, rendered as plain unified diff in a split pane — no syntax highlighting in v1 (real scope, deferred), but genuinely a better fit than Xirp's own full-screen-only diff view per the source review's own complaint, worth framing as a place this project can beat Xirp's UX rather than just match it.

**Explicitly deferred, not force-fit into a TUI**: embedded browser (substituted with a one-line "detected: localhost:8000 — press to open in default browser" signal, which captures most of the real value); in-app file editor (substituted with "open in $EDITOR," already a CAPABILITIES.md-documented Xirp pattern under its own Extensibility features, and strictly better economy than reimplementing an editor).

---

## Session hooks / extensibility (Phase 4+, not earlier)

Two layers matching CAPABILITIES.md's own split: (1) installing CLI hook config that calls back into `sessionmgr __hook-fire` (Claude Code first, per the adapter tiering above), and (2) outbound webhook dispatch on `NeedsInput`/`Finished`/`SubagentFinished` — closing a gap CAPABILITIES.md documents Xirp itself leaving open (its reviewer had to bolt on Zapier).

**Secret-scrubbing is a day-one requirement for this feature specifically**, not deferred polish — CAPABILITIES.md documents Xirp's own lack of transcript redaction as a real, named gap. The webhook payload is deliberately minimal (no transcript content in v1) to sidestep the redaction problem rather than half-solve it. **Corrected per adversarial finding #11**: `worktree_path` in the payload must be sent relative to a configured project root, not as an absolute path — an absolute Windows path leaks the local username (`C:\Users\<name>\...`), which the "deliberately minimal" framing doesn't actually achieve as originally scoped.

**`__hook-fire` must fast-path no-op on any session id it doesn't recognize** (adversarial finding #13) — a globally-installed Claude Code hook fires on *every* Claude Code session on the machine, including ones launched entirely outside sessionmgr. An unrecognized session id must never trigger the daemon's auto-start sugar or block, or a manually-launched Claude Code session (nothing to do with this tool) would hang waiting on a hook callback with nowhere to report to.

---

## Testing strategy

Two tiers, mirroring `rusty_prime_agent` directly:

- **Unit tests, `sessionmgr-core`**: the `Session` state machine (`Created -> Running -> (NeedsInput | Errored) -> Merged | Discarded`) against fake `GitPort`/`ProcessPort`/`AgentAdapterPort` implementations — no real git, processes, or filesystem. Every transition edge case (double-close, close-while-needs-input, dependent-session-parent-never-finishes) gets fast, deterministic coverage here.
- **Black-box subprocess tests, workspace `tests/`**: `tests/common/mod.rs` ports `rusty_prime_agent`'s pattern directly — real compiled binary, isolated temp state root, hand-rolled `TempDir` (no `tempfile` crate dependency, matching the minimal-deps preference). `supervisor_restart_recovery.rs` is the acceptance test for the entire persistence design (§ Process supervision) and should exist and pass by the end of Phase 1, not as an afterthought. `worker_crash_recovery.rs` proves the "no silent respawn" rule.
- **Gated agent-CLI adapter tests**: real Claude Code/Codex/Gemini subprocess tests, skipped cleanly (not failed) when a given CLI isn't installed on the test machine — these are what actually validate the `needs_input` heuristics against real output, kept separate from the always-runnable suite so CI isn't hostage to three external CLI installs existing everywhere.

---

## Phased milestones (checkpoint-gated, per adversarial finding #12)

Ordered to de-risk the two highest-uncertainty items first, and **explicitly gates later-phase commitment on what earlier spikes find** rather than presenting the whole roadmap as equally committed regardless of outcome.

- **Phase 0 (blocking prerequisite)**: confirm `rusty_tokio` builds cleanly against this machine's real toolchain at a fresh pinned rev (not blindly reusing `rusty_prime_agent`'s older pin). Record the decision (rusty_tokio confirmed, or fallback to plain `tokio`) before any domain code is written.
- **Phase 1 (walking skeleton, two spikes)**: `SessionKind::PlainTerminal` only, Claude Code only, no worktree, no TUI. Proves the daemon/worker/detached-persistence loop (`supervisor_restart_recovery.rs` passing is the exit criterion) *and* runs the PTY-vs-piped-stdio spike and the Claude-Code-hooks-when-headless spike. Both spike results directly shape Phase 3's adapter design — do not proceed to Phase 3's hook-dependent design until this phase's hook spike has an answer.
- **Phase 2**: worktree isolation, all three `SessionKind`s, worktree lifecycle tests, Windows path-length/Defender smoke tests. Still Claude Code only, still CLI-only (no TUI) — matches SCOPE.md's original "de-risk core logic before UI" reasoning, which remains sound even at the expanded scope.
- **Phase 3 (gated on Phase 1's hook spike)**: Codex and Gemini adapters. Before this phase's design work starts, resolve whether either CLI has any hook-equivalent — if not, commit explicitly to the degraded-confidence pattern-matching path for that CLI rather than discovering it mid-implementation.
- **Phase 4**: TUI (grid, panes, diff view, command palette) + session hooks/webhook extensibility, including the secret-scrubbing boundary and the `__hook-fire` no-op requirement.
- **Phase 5**: dependent sessions (real, independent scope). Fork is *not* in this phase (see below).
- **Phase 6+ (gated on a dedicated research spike)**: switch-agent-mid-session and Fork together, both blocked on the same unresolved question — whether any of the three CLIs' `--resume`/`--continue`-equivalent flags can accept externally-supplied prior state, and in what format. Cost/model routing is explicitly deprioritized further — CAPABILITIES.md itself flags this capability as sourced only from restated marketing copy, not confirmed hands-on by any source, so re-verify it's a real, distinct feature (not just the already-confirmed mid-session agent-switch described from a cost angle) before committing any design effort to it.

---

## Risk list (final, rebalanced per adversarial findings #5 and #9)

1. **Claude Code hooks firing headless from a detached Windows-spawned process — unverified, promoted to a Phase-1 go/no-go gate**, not a background risk (was previously under-weighted relative to its actual blast radius on 2/3 of day-one CLIs once Codex/Gemini's own hook support is also unknown).
2. **Codex/Gemini hook mechanisms entirely unresearched** — resolve before Phase 3 design, not during it.
3. **ConPTY-attached child survival across an unclean worker crash** — unverified, rustils' PTY path was built for interactive foreground use, not detach-and-outlive.
4. **`rusty_tokio`'s long-term availability/stability** — confirmed reachable at planning time; Phase 0 exists specifically to convert that into a real, pinned, verified-building dependency before anything else depends on it.
5. **Windows local IPC transport (`AF_UNIX` via `rusty_tokio`) — de-risked, not open**, per direct evidence in `rusty_prime_agent`'s own shipped, tested Windows `AF_UNIX` usage (corrected from the original design's flatter, more cautious framing).
6. **`git worktree` Windows path-length/case-sensitivity gotchas** — mitigated (short ids, `longPathAware` manifest) but not eliminated; Phase 2 acceptance-tested, not assumed away. **Status update (`docs/phase-2-windows-verification.md`)**: the manifest is wired in and verified embedded, but it only ever covered this binary's own I/O — it has no effect on the `git.exe` child process. Measured on a real Windows box: `core.longpaths` (now forced on every git invocation) fixes some deep-path failures but not `git worktree add` itself, which hits a lower, Git-for-Windows-internal ceiling (~190-210 characters for the full worktree path) that no configuration this project controls can move. Downgrade from "mitigated" to "partially mitigated, ceiling measured and documented."
7. **Windows Defender/AV interaction with heavy concurrent worktree file I/O** — a genuinely Windows-native risk absent from the original design's risk list; Phase 2 smoke-tests at Defender's default settings. **Status update**: run, per `docs/phase-2-windows-verification.md` — 24 concurrent worktree sessions under real-time protection, no failures, no hangs, full cleanup. Closed as tested.
8. **Output pattern-matching fragility** — permanent, standing maintenance cost for any CLI without hook support, not a one-time unknown to resolve and forget.
9. **Fork and switch-agent depend on an unproven primitive** (per-CLI acceptance of externally-supplied prior state) — both explicitly gated in Phase 6+ rather than assumed buildable on schedule.

---

## Verification

- **Per-phase acceptance**: each phase above has a stated exit criterion (Phase 0: pinned dependency builds; Phase 1: `supervisor_restart_recovery.rs` passes + both spikes have an answer; Phase 2: worktree lifecycle tests pass + Defender/path-length smoke tests pass; Phase 3: gated adapter tests pass for whichever CLIs are installed on the dev machine).
- **Running it for real**: `cargo build --workspace`, then `sessionmgr new --agent claude-code` against a scratch repo, confirm the session survives `taskkill /IM sessionmgr.exe` (simulating the manager app being closed) followed by `sessionmgr daemon` + `sessionmgr list` showing it still running — this is the single behavioral proof the whole persistence architecture exists to deliver, and should be run manually at the end of Phase 1 in addition to the automated `supervisor_restart_recovery.rs` test.
- **Test suite**: `cargo test --workspace` runs the unit tests and the always-on black-box tests; gated adapter tests run separately (`SESSIONMGR_TEST_CLAUDE_CODE=1 cargo test`, etc.) on whichever CLIs are actually installed.

## Critical files/patterns being reused (not reinvented)

- `C:\dev\rusty_prime_agent\src\worker\mod.rs` — the re-exec-with-detached-spawn pattern this project's daemon/worker split is modeled on directly.
- `C:\dev\rusty_prime_agent\src\procutil.rs` — `prepare_detached`/`is_alive`/`kill`, the exact primitives `sessionmgr-proc` ports.
- `C:\dev\rusty_prime_agent\Cargo.toml` — the `[lib]`+`[[bin]]` split pattern this plan applies via the new `sessionmgr-protocol` crate.
- `C:\dev\rusty_prime_agent\tests\common\mod.rs` — the black-box subprocess test harness pattern.
- `C:\Users\baileyrd\.buzz\REPOS\rustils\crates\platform-windows\src\pty.rs` — the ConPTY implementation the TUI's session panes reuse instead of adding a second PTY crate.
- `C:\Users\baileyrd\Downloads\xirp\CAPABILITIES.md` — the feature target every phase above is scoped against.
