# Phase 7 report — Switch agent mid-session

PLAN.md's Phase 6+ groups Fork and switch-agent-mid-session under the
same ADR-0003 gate. Phase 6 shipped Fork for Claude Code; this phase
covers the other half — **switch-agent-mid-session, for every agent this
tool supports**. Unlike Fork, switch-agent needed no per-CLI deferral:
the design chosen here works uniformly for Claude Code, Codex, and
Gemini CLI on day one, though only Claude Code could be exercised
against a real, authenticated process in this environment when this
phase was first written. See "Update (2026-08-17)" below for what
changed once real Codex/Gemini credentials became available.

## Status

| Item | Outcome |
|---|---|
| `SessionStatus::SwitchedAway`, `Session.switched_from` | **Done** |
| `sessionmgr_agents::handoff::render_handoff` | **Done** |
| `Request::SessionSwitchAgent`, `sessionmgr switch-agent <id> <agent>` | **Done** |
| Works for every agent (Claude Code, Codex, Gemini CLI) | **Done** — no per-CLI deferral needed |
| Live verification | **Partial**, revised 2026-08-17 — see the update below for what real Codex/Gemini credentials did and did not resolve |

## The design

### Why this is not a per-CLI schema translation

ADR-0003's own consequences section named the real risk switch-agent
carries that Fork does not: Fork operates on a session the *same* CLI
already produced, so no translation is needed. Switch-agent crosses
CLIs — Claude Code's Messages-API-shaped JSONL, Codex's `RolloutItem`
enum, Gemini's `{type, content}` records — and there is no shared
schema between them. Building and maintaining three schema parsers (six
directional pairs, if translation went CLI-to-CLI rather than through a
common form) is a maintenance burden with no verification path in this
environment for two of the three formats, and PLAN.md itself already
sanctions a specific, honestly-labeled fallback for exactly this
situation: "a summarized system-prompt injection from the prior
transcript, not implied to preserve full state."

So this phase builds that fallback, not a translator. The target
agent's own adapter method already accepts free-form initial-prompt
text (`AgentAdapterPort::launch_args`'s `extra` argument — an ordinary
CLI argument every adapter has accepted since Phase 3). Switch-agent
renders the source session's real transcript to plain text and hands
it to the new agent as that same `extra` argument, worded as an
explicit handoff ("you are taking over... below is that assistant's
own transcript") rather than presented as if it were native resumed
state. This is why the feature needed **no new `AgentAdapterPort`
method at all** — every existing adapter, including Codex's and
Gemini's own unverified-for-Fork adapters, already supports receiving
a handoff, because all three accept an initial prompt.

### In-place agent swap, not a resurrection

`sessionmgr-core`'s own transition table already anticipated this
design before this phase started — `Session::can_transition_to`'s
comment on `(Finished | Errored | Crashed, _) => false` reads: "Phase
6+'s fork/switch-agent work is a *new* session seeded from an old one,
never a resurrection of this record." Switch-agent follows that:
`source_id`'s own record is never mutated back to `Created` and
respawned. Instead:

1. The source's live process is stopped (the same graceful-then-forced
   sequence `session_close` already uses, factored out into
   `Supervisor::stop_worker` so both share it).
2. A **new** `SessionId` is minted.
3. The source transitions to a new terminal status, `SwitchedAway`, and
   is written first — so a crash between steps never leaves two
   live-looking records pointed at the same workspace.
4. The new session is created and its worker spawned.

### Same workspace, not a new worktree

Fork deliberately branches a new worktree from the source's branch tip,
because forking produces two independent lines of work. Switch-agent
is the opposite: CAPABILITIES.md describes it as "work continues
seamlessly" — one line of work, continuing under a different CLI. So
the new session reuses `source.workspace` verbatim rather than calling
`GitPort::worktree_add` at all. This is also why `SwitchedAway` needed
its own status rather than reusing `Closed`/`Merged`/`Discarded`:
unlike every existing close outcome, a switched-away session's
workspace is deliberately **not** disposed of — `session_close`'s
`dispose_workspace` step is skipped entirely, since the new session now
owns that same directory. `Session::switched_from` is consequently a
third relationship alongside `parent_id` and `forked_from`, not a reuse
of either — a dependent session shares a workspace but waits for its
parent; a switched-to session shares a workspace and does not wait,
because its source has already stopped by the time it exists.

This also made switch-agent kind-agnostic in a way Fork is not: Fork
requires `SessionKind::Worktree` because branching a new worktree only
makes sense for one. Switch-agent never branches anything, so it works
for any kind that can carry an agent at all.

### Rendering the transcript

`sessionmgr_agents::handoff::render_handoff` feeds the source session's
raw transcript bytes (`transcript.jsonl`'s concatenated `Output`
events) through a `vt100::Parser`, the same "interpret, don't print"
engine `pattern_watch::ScreenWatcher` already uses for `needs_input` —
and for the same reason: naively stripping ANSI escapes runs
cursor-positioned text together with no separating whitespace
(`pattern_watch`'s own module docs have the measured example, and this
module's own test reproduces it). Unlike `ScreenWatcher`, which only
ever needs the current screen and carries zero scrollback by design,
this needs the *whole* conversation. There is no cheap way to read "all
scrollback plus the current screen" back out of `vt100` in one pass, so
this instead renders into a virtual terminal tall enough (4000 rows) to
hold the whole thing without scrolling anything off — simpler and just
as correct for a one-shot render as juggling scroll offsets would be.
Both the input (last 512 KiB of raw transcript bytes) and the output
(4000 rows) are capped deliberately, not as an accidental side effect —
older history is dropped in favor of the most recent context.

## Live verification

This environment has real, authenticated `claude` access but not
`codex`/`gemini` (both installed, neither logged in — confirmed by
`codex login status` reporting "Not logged in" and `gemini --version`
succeeding without ever reaching an authenticated call). That shapes
what could actually be proven here versus what is asserted from design
alone:

- **Bookkeeping mechanics, live-tested with real Claude Code as the
  source**: `switch_agent_end_to_end_keeps_the_same_workspace_and_hands_off_the_transcript`
  starts a real `--agent claude` session, waits for it to reach
  `needs-input`, and switches it to `codex`. When the source reaches a
  live state in time, this proves — against real processes, not just
  unit tests — that the new session lands in the source's own
  workspace/branch, the source itself becomes `switched-away`, and the
  new session actually produces output (i.e., a real process started
  with the handoff text as its prompt).
- **Same sandbox limitation this project has documented since Phase 5**:
  this specific environment does not reliably drive an interactive
  `claude` session to `needs-input` within a 60-second test timeout
  (`agent_needs_input_claude.rs`, Phase 5's own report, and Phase 6's
  `fork_end_to_end_through_the_real_command` all hit this identically).
  The end-to-end test above is written to skip cleanly, with an
  explanatory message, rather than assert past it — and did skip when
  run for this phase. The three always-run validation tests (agent
  presence, same-agent rejection, live-conversation requirement) do not
  depend on reaching an interactive prompt at all and ran for real.
- **What remains genuinely unverified**: that Codex or Gemini CLI, on
  the *receiving* end of a handoff, actually produces a sensible
  continuation. Proving that needs real credentials for at least one of
  them, which this environment does not have — the same gap issues #14
  and #15 already name for Fork. Filing a third issue for this specific
  gap was considered and rejected: it is the identical missing
  precondition (real Codex/Gemini credentials somewhere this repo is
  also checked out), not a new piece of missing work.

## Tests

- `sessionmgr-core`: 3 new transition-table tests
  (`a_live_agent_conversation_can_be_switched_away`,
  `a_session_with_no_live_conversation_cannot_be_switched_away`,
  `switched_away_is_terminal_and_not_reclosable`) plus the existing
  `dependency` module's `Ready`-classification test extended to cover
  `SwitchedAway`.
- `sessionmgr-agents`: 4 new tests for `render_handoff` — plain
  rendering, the same cursor-positioning defect `pattern_watch` guards
  against, input-cap behavior (only the most recent bytes survive), and
  an empty-transcript edge case.
- `sessionmgr-protocol`: round-trip test extended with
  `Request::SessionSwitchAgent`.
- `crates/sessionmgr-daemon/tests/switch_agent.rs` — 4 new black-box
  tests: 3 always-run (agent required, same-agent rejected, live
  conversation required) and 1 live-gated end-to-end test against real
  `claude` (skipped cleanly in this run; see Live verification above).
- `cargo test --workspace` green (aside from the one pre-existing,
  documented interactive-PTY timeout also present on unmodified `main`)
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo fmt --all --check` clean
- `cargo +1.88 check --workspace --all-targets` (MSRV) clean
- `cargo check --target x86_64-pc-windows-msvc` — not run in this
  environment (missing MSVC cross-linker for a transitive dependency's
  build script, the same pre-existing environment limitation Phase 5's
  report first confirmed unrelated to this project's own code); nothing
  in this diff touches Windows-specific code

## What is not done

- **A full, automated, both-legs-in-one-run proof that Codex or Gemini,
  as the *receiving* agent, recalls a fact from the handoff.** See the
  2026-08-17 update below: this was attempted for real, with real
  credentials, and got close — one manual round trip succeeded outright,
  and the automated test mechanism itself is confirmed correct — but no
  automated run completed both legs before hitting a real, external
  account-level quota wall on both providers. The tests are written and
  gated to skip cleanly rather than fail when that happens, matching this
  suite's own established convention for exactly this class of
  environment-dependent gap.
- **Hook reinstallation across a switch.** A session created with
  `--hooks` has a hook config file installed for its *original* agent
  (e.g. `.claude/settings.json`). Switch-agent does not reinstall an
  equivalent config for the new agent — the new session runs with
  `hooks: false` unconditionally. A user who wants webhook/status-hook
  coverage after switching needs to reinstall it manually today. Not
  filed as a separate issue: it is a narrow, well-understood gap in an
  already-narrowly-scoped v1, not a piece of unknown work.
- **Summarization, as opposed to verbatim-tail rendering.** The handoff
  is the source's real transcript, tail-capped by byte count, not an
  LLM-generated summary. This was a deliberate choice (no recursive
  dependency on spawning another agent just to compress context) but
  means a very long prior conversation loses its *earliest* content
  rather than its least relevant content. Acceptable for a v1 matching
  PLAN.md's own sanctioned fallback wording; a smarter compaction
  strategy is future work, not a defect in this one.
- **TUI support.** Not mentioned in this report's original scope, but a
  real gap: switch-agent shipped as a daemon feature and CLI subcommand
  (`sessionmgr switch-agent <id> <agent>`) only, with no TUI-side way to
  reach it. Closed 2026-08-17 alongside the equivalent Fork gap — see
  `docs/phase-6-report.md`'s own "Update (2026-08-17, later still)"
  section for the full account (issue #22).

## Update (2026-08-17): live verification with real Codex/Gemini credentials

`OPENAI_API_KEY` and `GEMINI_API_KEY` were configured in this
environment for the first time, specifically to close the gap this
report's original "What is not done" named. `codex login --with-api-key`
succeeded and `gemini` worked directly (it reads the env var with no
separate login step). This did not fully close the gap, but for a more
interesting reason than "still no credentials."

### What changed in the test suite

`crates/sessionmgr-daemon/tests/switch_agent.rs` gained a third test
tier: four `*_recalls_codeword_from_the_rendered_handoff` tests
(`gemini_source_claude_target`, `claude_source_gemini_target`,
`codex_source_gemini_target`, `gemini_source_codex_target`), covering
every source/target combination this report's original gap named
(Codex/Gemini as source, and as the *receiving* agent). Unlike the
existing bookkeeping-tier test, these drive each CLI directly through
`--kind same-dir --no-pty` (mirroring `fork_sessions.rs`'s own
mechanism-level codeword test, not the full interactive `switch-agent`
command), call the real `sessionmgr_agents::render_handoff` production
code, and assert that a *different* CLI, given only the rendered text as
its initial prompt plus one explicit recall question, can extract a fact
planted in the source's own transcript.

Two real bugs surfaced and were fixed while building this:

- **The first version seeded the codeword through the *prompt*** ("Remember
  this codeword, reply OK"), not the reply. `render_handoff` only ever
  renders a session's own *output* bytes — a one-shot, non-interactive
  `-p`/`exec` invocation prints just its final answer, with no echo of
  what it was asked, unlike an interactive terminal. The rendered handoff
  therefore contained only "OK", and no CLI could have recalled anything
  from it — confirmed by an actual failing test run, not caught by
  review. Fixed by making the codeword the assistant's own visible
  reply instead.
- **`claude -p` and `codex exec`, run through this project's own
  `--no-pty` piped-stdio backend, both wait on stdin** before proceeding
  (`claude` prints "no stdin data received in 3s, proceeding without it"
  and continues; `codex exec` appeared to wait indefinitely) — because
  `Backend::Piped` (`worker.rs`) never closes a session's stdin pipe on
  its own, correct for an interactive session `attach` might still write
  to, but not what a one-shot invocation needs. Fixed inside the test
  file only (not production code, which never launches these CLIs'
  one-shot modes) by wrapping each command in `sh -c '... < /dev/null'`,
  passing the prompt as `$1` rather than interpolating it into the
  script string, since a rendered handoff can contain arbitrary
  characters.

### What ran live, and what didn't

The always-run and bookkeeping-tier tests all passed, including —
notably, unlike when this report was first written —
`switch_agent_end_to_end_keeps_the_same_workspace_and_hands_off_the_transcript`,
which actually reached `needs-input` against a real `claude` process this
time rather than skipping on the sandbox's interactive-PTY limit.

The four new recall tests did not complete a full run in this
environment, but not for lack of credentials:

- **Codex**: manual `codex exec` probes run directly outside the
  automated suite consistently returned `ERROR: Quota exceeded. Check
  your plan and billing details.` — a real, authenticated account with
  no usable billing quota. Confirmed reproducible, not transient
  (retried multiple times over several minutes). **Correction, same
  day** (see `docs/phase-6-report.md`'s own second 2026-08-17 update for
  the full account): `codex_credentialed()`, the gate the two
  Codex-involving tests (`codex_source_gemini_target_...`,
  `gemini_source_codex_target_...`) used, had a real bug at the time
  this paragraph was first written — it checked only stdout, but `codex
  login status` writes its message to *stderr* when run without a TTY,
  so the check always read as "not logged in" regardless of real state.
  That means these two tests were actually skipping on the *gate*, not
  on a live-attempted quota error, even though the quota block itself
  was genuinely real (confirmed by the manual probes above, run
  independently of the buggy gate). Fixed in the Codex Fork work; a
  re-run afterward confirmed both tests now correctly reach their real
  live call and skip on the genuine quota error, matching what the
  Gemini-only test below already did honestly from the start.
- **Gemini**: worked initially — one standalone `gemini --skip-trust -p`
  call outside the automated suite genuinely recalled a codeword from a
  rendered handoff, real end-to-end proof the mechanism works — but this
  session's own testing (many manual probes plus the automated tests'
  own calls) exhausted the API key's free-tier quota
  (`generativelanguage.googleapis.com/generate_content_free_tier_requests,
  limit: 20, model: gemini-3.5-flash`) partway through. Later attempts,
  including a second full test-suite run after waiting out the quota
  error's own suggested retry delay, still failed the same way. Unlike
  Codex, this test's own gate (`gemini_credentialed`, a plain
  `GEMINI_API_KEY` env-var check) had no bug — it genuinely reached the
  live call and skipped on the real timeout, both before and after the
  Codex gate fix above.

Both are real, external, account-level constraints this project does not
control — not a sessionmgr defect, and not the same "no credentials at
all" gap this report originally documented. The four tests are gated
(`codex_credentialed`/`gemini_credentialed`, checking real login/env-var
state — see the correction above for `codex_credentialed`'s own bug and
fix) and skip cleanly rather than fail when a live call doesn't finish
in time, the same "environment-dependent, not asserted here" convention
this suite's other live-CLI tests already use — so they skipped
correctly here rather than reporting a false failure, and will run for
real wherever these two accounts have usable quota.

### Issues #14/#15: same credentials, applied to Fork instead

The same live credentials were used to revisit issues #14 and #15 (see
each issue's own 2026-08-17 comment for full detail, as first written).
At the time this paragraph was originally written, neither was closed —
both appeared to need a change to `Supervisor::session_fork` itself (it
gated every fork on `source.native_session_id` already being recorded,
which fit Claude Code's id-based mechanism but not Codex's
discover-after-spawn one or Gemini's path-based one at all), which
looked bigger than either issue originally scoped.

**Stale as of later the same day — both are now closed.** That
`session_fork` change did ship, in `sessionmgr-core::ports::ForkSource`
and the `session_fork` update accompanying it, and both issues closed
for real: Gemini CLI Fork (issue #15) once its file-location question
resolved into a plain `projects.json` registry read, and Codex Fork
(issue #14) once its own "post-spawn filesystem watch" framing turned
out to be an unnecessary assumption — Codex's rollout file already
exists by fork time, so its discovery is a lazy lookup exactly like
Gemini's, not a watch. See `docs/phase-6-report.md`'s own two dated
2026-08-17 updates for the full account of what shipped, what could and
could not be live-verified for each, and a real credential-check bug
(`codex_credentialed()`'s stdout/stderr issue, also relevant to this
report's own "What ran live, and what didn't" section above) found and
fixed along the way.
