# Phase 6 report — Fork (Claude Code)

PLAN.md's Phase 6+ groups switch-agent-mid-session and Fork together,
both gated on ADR-0003's own spike. That spike answered "yes, all three
CLIs accept externally-supplied prior state" — but not "sessionmgr
already knows how to reach every one of them for real." This report
covers the first slice of that gap: **Fork, for Claude Code**, built and
live-verified. Codex and Gemini CLI are explicitly deferred, each with a
filed issue naming exactly what is missing — not silently dropped, and
not guessed at. Switch-agent-mid-session is untouched; see "What is not
done."

## Status

| Item | Outcome |
|---|---|
| `Session.native_session_id`/`forked_from` | **Done** |
| `AgentAdapterPort::supports_fork`/`fork_args`/`launch_args`'s `native_id` | **Done** |
| `GitPort::worktree_add`'s `start_point` (fork-from-branch-tip) | **Done** |
| `Request::SessionFork`, `sessionmgr fork <id>` | **Done** |
| Claude Code adapter (`--session-id` pin, `--resume --fork-session`) | **Done, live-verified** |
| Codex adapter | **Done**, 2026-08-17 — see this report's own second dated update below |
| Gemini CLI adapter | **Done**, 2026-08-17 — see this report's own dated update below |
| Live verification | **Done** — real mechanism proven, plus an honest limit on what could be proven in this environment |
| TUI command palette action | **Done**, 2026-08-17 — see this report's own third dated update below (issue #22) |

## The design

### Why Fork needed a native session id sessionmgr didn't have before

ADR-0003 proved externally-supplied prior state is acceptable to all
three CLIs. It did not answer a question that turned out to matter just
as much: **how does `sessionmgr` know *which* native session to hand
back later?** Before this phase, a Claude Code session launched via
`sessionmgr new --agent claude` got whatever session id Claude Code
itself chose internally — sessionmgr never captured it, because nothing
needed it yet.

Two ways to close that gap were weighed:

- **Pin at launch** — generate a real UUID before spawning, pass it as
  `--session-id <uuid>`, record it immediately. No filesystem
  discovery, no race window, no adapter-side I/O (adapters stay the
  pure, zero-I/O format-producers they already were).
- **Discover after spawn** — let the CLI pick its own id, then locate
  it after the fact by scanning the CLI's own state directory (a
  worktree's own project directory is guaranteed empty until a session
  starts in it, so "the first file that appears" is unambiguous).

**Pin was chosen, live-verified as viable, and shipped.** Discovery was
seriously considered — it is, in fact, exactly what Codex needs, since
Codex has no equivalent pinning flag at all (confirmed absent from a
real `codex --help`) — but for Claude Code, pinning is strictly simpler
and was already known to work: ADR-0003's own spike ran `claude
--session-id <uuid> -p "..."` and got a real transcript at that exact
id. This phase extended that finding one step further, live, in this
same environment: **`--session-id` also pins the *target* id of a
`--resume ... --fork-session` call**, not only a fresh session — see
"Live verification" below. That is what let native-id pinning become
`sessionmgr`'s single, uniform mechanism for Claude Code rather than
needing two different code paths for "create" and "fork".

Pinning is applied to **every** Claude Code session, not only ones
created with some `--fork`-anticipating flag — `AgentAdapterPort::supports_fork()`
is the creation-time question this decision turns on ("should I bother
pinning, before anyone has asked to fork anything"), kept deliberately
separate from `fork_args()` ("give me the actual fork command"), which
answers a different question at a different time. `sessionmgr-agents`'
own test suite asserts the two never disagree for any adapter, so they
cannot drift silently.

### `parent_id` (Phase 5) and `forked_from` (this phase) are not the same relationship

A dependent session (Phase 5) *shares* its parent's workspace — same
cwd, same branch, same files, deliberately. A forked session shares
none of that: it gets its **own** independent worktree and branch, and
only the *conversation history* carries over. Reusing `parent_id` for
both would make `SessionKind::Dependent` mean something different
depending on which relationship was meant — exactly the ambiguity
ADR-0003 itself flagged as needing an answer before this phase started.
`forked_from: Option<SessionId>` is its own field for exactly that
reason.

### Forking branches from the source session's own tip, not the repo's default branch

`GitPort::worktree_add` gained an optional `start_point: Option<&str>`
(git's own `<commit-ish>` argument to `worktree add -b`). An ordinary
new worktree session still passes `None` (git's own default: whatever
`HEAD` the main repository has checked out). A forked session passes
`Some(&format!("sessionmgr/{source_id}"))` — the source session's own
branch — so the new worktree's **code state** matches the **conversation
history** it starts with. Getting this wrong would mean a forked agent
finding itself "remembering" edits that are not actually on disk in its
own new worktree; this is the one piece of Fork that has nothing to do
with the agent CLI at all and would have been just as necessary even if
Fork only ever cloned plain shell sessions.

### Validation order in `Supervisor::session_fork`

Checked in this order, each for a concrete reason rather than
defensive habit:

1. `source.kind == Worktree` — only a worktree-owning session has a
   branch to fork from at all.
2. `source.agent.is_some()` — nothing to fork without a conversation.
3. `adapter.supports_fork()` — as of this phase, Claude Code only; the
   error names the agent and points at this report.
4. `source.native_session_id.is_some()` — absent for a session created
   before this phase, or whose adapter did not support pinning at the
   time.
5. `source.workspace`/`workspace.branch` exist.
6. `sessionmgr_core::parent_readiness(source.status) != Unavailable` —
   **direct reuse of Phase 5's own function**, unchanged: "does this
   session's git state still exist" is the identical question a
   dependent session's wait-for-parent already asks, just applied to a
   different relationship (fork lineage instead of workspace-sharing).
   No new domain logic was needed for this check at all.

## Live verification

Real, not simulated — this environment happens to carry a genuinely
authenticated `claude` install, and that was used directly rather than
reasoning from `--help` text alone wherever it could be.

**The core mechanism, proven end to end**, through the real black-box
test suite (`resume_fork_session_id_actually_preserves_conversation_context`
in `crates/sessionmgr-daemon/tests/fork_sessions.rs`):

1. A session pinned to `uuid1` was told (via `-p`, non-interactively):
   "Remember this exact codeword: ORACLE42. Reply with only the word
   OK." It replied `OK` and finished.
2. A **second, independent** session was created: `claude --resume
   uuid1 --fork-session --session-id uuid2 -p "What was the codeword?
   Reply with only the codeword."` It replied `ORACLE42` — the exact
   value from the first session's own conversation, which the second
   session's own process never received in its prompt.
3. Both transcript files were confirmed on disk at exactly their
   pinned ids (`~/.claude/projects/.../uuid1.jsonl` and
   `.../uuid2.jsonl`) — the pin was not incidental, both the source and
   the *forked* session's own new id landed exactly where asked.

This is stronger evidence than ADR-0003's own spike had for this
specific claim: the spike confirmed *a* session's id could be pinned;
this phase confirms the **forked** session's id can be pinned too,
which is what lets `sessionmgr` avoid discovery machinery entirely for
this adapter.

**The daemon-side validation rules** (kind/agent/support checks) are
covered by always-run black-box tests needing no credentials at all —
`forking_requires_a_worktree_session`, `forking_requires_an_agent`,
`forking_an_unsupported_agent_names_the_gap_clearly`.

**What could not be fully verified here, and why that is stated rather
than glossed over**: `fork_end_to_end_through_the_real_command` drives
the actual `sessionmgr fork <id>` command through the real `--agent
claude` **interactive** path (not `-p` mode). It did not reach a live
state within its own 60-second timeout in this environment — the exact
same interactive-PTY behavior `agent_needs_input_claude.rs` already
documents, reproduced identically against unmodified `main` before this
phase started (see `docs/phase-5-report.md`'s own note on this). The
test is written to skip cleanly, not fail falsely, when this happens,
and does not assert anything about it — it is not evidence Fork's
end-to-end path is broken, only that this specific environment's
interactive-PTY behavior is not something either this phase or Phase 5
could newly resolve. The mechanism test above is what actually carries
this phase's confidence; the end-to-end test is additional coverage for
whichever environment (a real Windows box, in particular) does not hit
that limitation.

## Why Codex and Gemini CLI were deferred, not guessed at (both closed 2026-08-17 -- see below)

Both had real fork mechanisms — ADR-0003 already established that. Both
were missing one specific, identified piece, not "everything":

- **Codex** (issue [#14](https://github.com/baileyrd/rusty_yirp/issues/14)): no flag to pin a new session's own native
  id at creation (confirmed absent from a real `codex --help`); its own
  id is always self-assigned. Reaching it needs *discovery* — a
  post-spawn scan of `CODEX_HOME/sessions/<Y>/<M>/<D>/rollout-*.jsonl`
  for the file that appears after spawning, which is workable (the
  filename embeds the id, and a fresh worktree's own directory has
  nothing else to guess at) but is a genuinely different kind of
  machinery than this phase built — a filesystem watch, not a pure
  format-producing adapter method — and there are no Codex credentials
  in any environment available while this phase was built to verify it
  against a real process. Codex's own `app-server` JSON-RPC protocol
  (`ThreadInjectItemsParams`, confirmed real via its own test suite in
  ADR-0003) is a plausible alternative path worth weighing against
  file-discovery when this is picked up, since it sidesteps needing to
  replicate Codex's own rollout-file format at all.
- **Gemini CLI** (issue [#15](https://github.com/baileyrd/rusty_yirp/issues/15)): `--session-file <path>` is the most
  *explicit* of the three mechanisms — an arbitrary file path, no
  native id needed — but that file has to be the source session's own
  current chat-history file, which lives under gemini-cli's own
  per-project temp directory, keyed by a hash of the working directory
  this phase did not reverse-engineer from the installed bundle (only
  confirmed the mechanism exists and roughly where the file lives, not
  the exact hashing algorithm). No Gemini credentials exist in any
  environment available here either.

Both are exactly the kind of gap this project's own conventions ask to
be filed, not shipped as a guess: `AgentAdapterPort::fork_args` returns
`None` for both, `AgentAdapterPort::supports_fork` returns `false` for
both, and `sessionmgr fork <id>` against either fails with a clear
error naming the agent — never a silent no-op, never a best-effort
attempt built on an unverified guess about either CLI's internal
storage format.

## Tests

9 new unit tests across `sessionmgr-agents` (launch-args pinning,
fork-args formatting, the `supports_fork`/`fork_args` drift-guard test
shared across all three adapters) and `sessionmgr-proc` (the hand-rolled
UUID v4 formatter, checked for RFC 4122 shape — version/variant nibbles,
group lengths — not just "it doesn't crash"). 5 new black-box tests in
`crates/sessionmgr-daemon/tests/fork_sessions.rs` (3 always-run
validation tests, 1 live-verified mechanism test, 1 live-attempted
end-to-end test that skips cleanly on the known environment limitation
above).

`cargo test --workspace` green (all always-run and live-gated Fork
tests pass; the pre-existing `agent_needs_input_claude`/`codex`/`gemini`
tests carry the same environment caveat every phase report since 5 has
noted). `cargo clippy --workspace --all-targets -- -D warnings`/`fmt
--all --check` both clean. `cargo +1.88 check --workspace --all-targets`
(MSRV) clean. `cargo check --workspace --all-targets --target
x86_64-pc-windows-msvc` could not run in this environment specifically
(missing MSVC cross-linker tooling for a transitive dependency's own
build script, unrelated to anything in this phase — confirmed identical
against unmodified `main` in `docs/phase-5-report.md`); nothing in this
phase's diff touches Windows-specific code (`sessionmgr-git`'s and
`sessionmgr-proc`'s changes are plain, `#[cfg]`-free Rust), so this is
recorded as an environment gap in this pass, not a claim the Windows
build was checked and found fine.

## What is not done

**Codex and Gemini CLI Fork support** shipped 2026-08-17, closing issues
#14 and #15 — see this report's own two dated updates below for the full
account, including what could and could not be live-verified for each.

**Switch-agent-mid-session** — entirely untouched by this phase.
ADR-0003 scoped the format each CLI expects, which is the real
prerequisite for translating one CLI's transcript into another's, but
no translation code exists yet. This remains real, separately-scoped
work, not started here.

**TUI support for Fork.** `SessionSummary.forked_from` is surfaced over
the wire and in `sessionmgr list`'s new `FORKED-FROM` column, matching
Phase 5's own precedent for `parent` — enough for a future TUI pass to
build a "Fork" command-palette action and lineage grouping on top of,
but no TUI-side UI was built in this pass. The command-palette action
itself was closed 2026-08-17 (issue #22) — see this report's own third
dated update below. Lineage grouping in the grid layout remains
unbuilt.

**Re-verifying the cost/model-routing capability.** PLAN.md's own
Phase 6+ note also asks for this before committing design effort to it.
Untouched by this phase; `CAPABILITIES.md` still flags it as sourced
only from restated marketing copy.

## Update (2026-08-17): Gemini CLI Fork implemented

`GEMINI_API_KEY` was configured in this environment for the first time
(see `docs/phase-7-report.md`'s own matching update), closing the
blocker issue #15 named: locating the source session's own current
chat-history file for `--session-file <path>`.

### The file-location question is solved -- and simpler than expected

No hash needed reverse-engineering after all. Reading the installed
`@google/gemini-cli` bundle directly (`chunk-32XQ54AJ.js`) turned up
`~/.gemini/projects.json` (or `$GEMINI_CLI_HOME/projects.json` when that
env var is set): a plain JSON registry mapping each project's own
absolute working directory to a short directory name, e.g.
`{"projects": {"/home/user/rusty_yirp": "rusty-yirp"}}`. `ProjectRegistry`'s
own key (`normalizePath`) is a plain `path.resolve`, lowercased only on
`win32` -- no realpath/symlink resolution -- which lines up exactly with
how `sessionmgr` already builds `Workspace.cwd` (from `git rev-parse
--show-toplevel`), so no path-normalization work was needed on this
project's own side either.

One wrinkle issue #15's own research did not know about: `tmp/<name>/chats/`
holds two shapes. A flat `session-<timestamp><shortid>.jsonl`, whose
first line reads `{"kind":"main",...}` -- the real top-level
conversation -- and a nested `<parent-id>/<subagent-id>.jsonl`, first
line `{"kind":"subagent",...}` -- a tool-driven sub-conversation, not the
one to fork. `GeminiCli::locate_current_chat_file` (`crates/sessionmgr-agents/src/gemini.rs`)
only considers the flat, `"kind":"main"` shape, picking the newest by
mtime.

### `AgentAdapterPort::fork_args` needed a real signature change, not just a Gemini-side fix

The design section above ("Validation order in `Supervisor::session_fork`")
described step 4 as `source.native_session_id.is_some()`, checked
*before* calling into the adapter at all -- true for Claude Code, but
wrong for Gemini CLI, whose actual mechanism is path-based and has no id
concept at all. Shipping Gemini Fork honestly needed:

- `ForkSource<'a>` (`sessionmgr-core::ports`): replaces `fork_args`'s
  bare `source_native_id: &str` parameter with a small struct carrying
  both `native_session_id: Option<&str>` and `workspace_cwd: &Path` --
  each adapter reads whichever field its own mechanism needs.
- `Supervisor::session_fork` no longer hard-requires `native_session_id`
  up front; it now always calls `fork_args` and lets the adapter decide.
- `AgentAdapterPort::supports_fork()` and `fork_args(..).is_some()` are
  **no longer asserted equal in both directions** by
  `sessionmgr-agents`' own test suite -- they still were, for a long
  time, because Claude Code's `fork_args` never failed for any reason
  other than "not supported". Gemini CLI breaks that: `supports_fork()`
  answers "does the mechanism exist" (`true`), while any single
  `fork_args` call can still return `None` for a real, session-specific
  reason (no chat file located yet for that workspace). The drift-guard
  test now asserts the direction that still universally holds --
  `!supports_fork() ⟹ fork_args(..).is_none()` -- and Gemini's own
  "finds it when it's there / doesn't when it isn't" behavior is covered
  by dedicated fixture-based unit tests instead.

### Live verification

Mixed, the same honest way this report's original "Live verification"
section was: `locate_current_chat_file`'s own discovery algorithm is
fully unit-tested against real captured JSON shapes (`gemini.rs`'s own
`ScratchGeminiHome`-based fixtures -- newest-file selection, subagent
files correctly excluded, missing-registry and no-chat-file-yet cases),
and the always-run black-box test
(`forking_a_gemini_session_with_no_conversation_yet_names_the_gap_clearly`)
passed for real. The new **content-preservation** mechanism test,
`gemini_session_file_actually_preserves_conversation_context`
(`fork_sessions.rs`), mirrors Claude Code's own codeword-recall pattern
but hit the same Gemini free-tier request quota
`docs/phase-7-report.md` already documents running out mid-session --
it skipped cleanly rather than asserting past it, the same tier every
other live-gated test in this suite uses. A standalone manual round trip
outside the automated suite *did* succeed earlier in this same session,
real end-to-end proof `--session-file` genuinely loads prior context (not
merely that the flag is accepted) -- recorded here rather than treated as
equivalent to the automated test actually passing.

### Tests

9 new unit tests in `crates/sessionmgr-agents/src/gemini.rs`
(`locate_current_chat_file`'s discovery logic against fixtures) plus
signature-update tests in `claude_code.rs`/`codex.rs`. The
`sessionmgr-agents` drift-guard test rewritten (see above). 2 new
black-box tests in `fork_sessions.rs` (1 always-run, 1 live-gated,
skipped cleanly here). `cargo fmt --all` (twice) clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace`
green aside from the same pre-existing, undiffed `agent_needs_input_claude`
interactive-PTY flake every phase report since 5 has documented, `cargo
+1.88 check --workspace --all-targets` (MSRV) clean.

### What is still not done

**Codex Fork (issue #14)** remains open as of this update -- believed at
the time to need a genuinely separate piece of machinery (a post-spawn
filesystem watch), wired into `Supervisor`/`worker.rs` rather than a
pure adapter method. **That assessment turned out to be wrong**; see
this report's own second dated update immediately below, from later the
same day, for why and what shipped instead.

## Update (2026-08-17, later the same day): Codex CLI Fork implemented

The "post-spawn filesystem watch" framing above assumed Codex's native
thread id had to be captured and recorded at *session-creation* time,
mirroring Claude Code's pin-at-launch design. Revisiting this after
Gemini CLI's own fork support shipped (this report's first 2026-08-17
update, directly above) showed that assumption was never actually
necessary: like Gemini's own chat-file lookup, Codex's rollout file
(with a usable `SessionMeta.cwd`/`id`) is live-confirmed to already
exist by the time anyone would ask to fork a real conversation -- it's
written before the model call even completes or fails (issue #14's own
prior comment). So this can be, and is, a **lazy, synchronous lookup at
fork time**, exactly like Gemini's -- no watch, no new async machinery,
no `Supervisor`/`worker.rs` changes beyond what `ForkSource` already
provides.

### The mechanism

`Codex::locate_native_thread_id` (`crates/sessionmgr-agents/src/codex.rs`)
scans `<codex_home>/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`, newest
day-directory first (plain lexicographic sort on the zero-padded path
components sorts chronologically for free), for the rollout file whose
first line's `SessionMeta.cwd` matches the source session's own
workspace. `codex_home_dir` resolves `CODEX_HOME` (real: `codex --help`
documents `$CODEX_HOME/<name>.config.toml`) or falls back to `~/.codex`,
confirmed live to be where rollout files land with `CODEX_HOME` unset --
the same default-fallback shape `gemini_home_dir` uses for
`GEMINI_CLI_HOME`. `Codex::fork_args` uses the discovered id to build
`codex fork <id>` (confirmed real via a real installed `codex fork
--help`), ignoring `new_native_id` entirely -- Codex has no way to pin
the *forked* session's own new thread id either, the same reason
`launch_args` already ignores `native_id`.

### Live verification

Mixed, the same honest way every other live-gated claim in this report
is: the discovery algorithm is fully unit-tested against real captured
`SessionMeta` shapes (`codex.rs`'s own `ScratchCodexHome`-based
fixtures -- newest-day-first ordering, non-matching-cwd and
non-rollout-file exclusion, no-sessions-dir-at-all). The always-run
black-box test
(`forking_a_codex_session_with_no_conversation_yet_names_the_gap_clearly`)
passed for real -- confirmed live while building it that a bare
interactive `codex` launch does not write a rollout file until an
actual conversation happens, so this test needs no credentials and costs
no quota either way. The new **content-preservation** mechanism test,
`codex_resume_actually_preserves_conversation_context_via_discovered_thread_id`
(`fork_sessions.rs`), uses `codex exec resume <id> <prompt>` rather than
`codex fork <id>` -- `codex exec` (the non-interactive mode this test
needs for deterministic, PTY-free driving) has no `fork` subcommand,
only `resume`, so this tests the same underlying context-preservation
claim Fork depends on without literally exercising `fork`'s own
lineage-tracking fields. It hit the same real, external billing-quota
block this report's Gemini update and `docs/phase-7-report.md` already
document (confirmed reproducible, not transient, throughout this
session) and skipped cleanly rather than asserting past it.

### A real bug found and fixed along the way

While gating this test on real Codex credentials, `codex login status`
turned out to write its "Logged in..."/"Not logged in" message to
**stderr**, not stdout, when run without a TTY attached (confirmed
live: `codex login status < /dev/null` produces empty stdout). The
`codex_credentialed()` helper this report's Gemini update introduced in
both `fork_sessions.rs` and `switch_agent.rs` only checked stdout --
meaning every Codex-gated test in both files was silently skipping with
"codex is not logged in" regardless of real credential state, and would
have kept doing so forever even once real credentials or quota became
available. Fixed in both files (check stderr too) as part of this
update; re-run afterward confirmed the previously-merged
`switch_agent.rs` tests now correctly reach their real live calls before
skipping on the genuine quota block, rather than skipping on the bug.

### Tests

9 new unit tests in `crates/sessionmgr-agents/src/codex.rs`
(`locate_native_thread_id`'s discovery logic against fixtures). 2 new
black-box tests in `fork_sessions.rs` (1 always-run, 1 live-gated,
skipped cleanly here on the quota block). One obsolete black-box test,
`forking_an_unsupported_agent_names_the_gap_clearly`, removed: it
verified that forking a Codex session failed with "Codex does not
support Fork yet", which stopped being true the moment this update
shipped -- `Supervisor::session_fork`'s own "{agent:?} does not support
Fork yet" branch has no real adapter left among this project's three to
exercise it against, so it stays in the source as defensive code for a
future, genuinely unsupported adapter, without a black-box test that
would need to fake one to run. The credential-check bug fix above, in
two files. `cargo fmt --all` (twice) clean, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo test --workspace` green
aside from the same pre-existing, undiffed `agent_needs_input_claude`
interactive-PTY flake every phase report since 5 has documented, `cargo
+1.88 check --workspace --all-targets` (MSRV) clean.

### What is still not done

Full content-preservation through a real, completed `codex fork` remains
unverified live in this specific environment -- not because the
mechanism is in doubt (the discovery algorithm is fully unit-tested, and
`codex exec resume`'s own equivalent context-preservation claim was
attempted live and is correctly gated to re-run automatically once
quota allows), but because this session's own OpenAI account has no
usable billing quota, confirmed reproducible throughout. Nothing further
is blocked architecturally; this is purely an external account-state gap
the next environment with working Codex credentials will close by simply
running the existing, already-correct test suite.

## Update (2026-08-17, later still): TUI command palette actions for Fork and switch-agent

Closes issue #22 ("TUI: add Fork and switch-agent actions to the command
palette"), filed as the direct successor to this report's own "TUI
support for Fork" gap above once both Codex and Gemini CLI Fork existed
for real (the two updates directly above) and it became clear the same
palette gap applied equally to Phase 7's switch-agent-mid-session, which
had never had TUI wiring at all, in any pass.

### What changed

`crates/sessionmgr-tui/src/app.rs`'s `PaletteAction` enum gained two
variants, `Fork` and `SwitchAgent`, alongside the existing
`NewSession`/`CloseFocused`/`Rename`/`Focus`. Both follow patterns the
palette already established rather than inventing new ones:

- **`Fork`** mirrors `Focus` and `CloseFocused` — no extra input needed,
  it applies immediately against the focused session. Wired straight to
  a new `client::session_fork` (always `pty: true`, the same "fast
  keyboard shortcut for the common case" reasoning `session_new`'s own
  palette action already uses — `sessionmgr fork <id> --no-pty` remains
  the way to opt out of a PTY).
- **`SwitchAgent`** mirrors `Rename` — it needs one piece of text (the
  target agent's name) it doesn't have yet, so selecting it opens a new
  `PromptKind::SwitchAgent(SessionId)` overlay, submitted through a new
  `client::session_switch_agent`.

Both new `client.rs` functions send `Request::SessionFork`/
`Request::SessionSwitchAgent` — protocol messages that already existed
from Phase 6/7, unchanged here. No daemon-side code was touched; this
is purely a new client role reaching an already-shipped surface, exactly
as issue #22 scoped it.

One small piece of real logic came with `SwitchAgent`: parsing the
prompt's free-text agent name (`claude`/`claude-code`/`codex`/`gemini`)
into an `AgentKind`. `sessionmgr-daemon`'s own CLI already has this
exact match arm (`parse_agent_name` in `lib.rs`, backing `sessionmgr
switch-agent <id> <agent>`), but this crate cannot depend on
`sessionmgr-daemon` — see `client.rs`'s own module docs on why that
boundary is deliberate, the same reasoning that already duplicates the
socket-framing code in this crate rather than sharing it. The three-line
match is duplicated rather than shared, with a doc comment saying so.

### Live verification

Driven with real keystrokes against a real `sessionmgr tui` process
under a real PTY (Python's `pty`/`os.write` standing in for `hub`'s
process control this session doesn't have access to — same bytes, same
socket path, same daemon), against a real, live, credentialed Claude
Code session:

1. **Fork**: focused a live `claude --session-id <id>` session (`Running`,
   native id already pinned at launch), opened the palette (`Ctrl-B k`),
   typed `Fork`, pressed Enter. `sessionmgr list` immediately after
   showed a brand-new session with `FORKED-FROM` set to the source id,
   running `claude --resume <id> --fork-session --session-id <new-id>`
   — the exact command Phase 6's own Claude Code adapter builds, reached
   this time from the TUI rather than `sessionmgr fork` directly.
2. **SwitchAgent**: focused a second live Claude Code session, opened
   the palette, typed `Switch`, pressed Enter to open the prompt, typed
   `gemini`, pressed Enter. The status line reported `<id> switched
   agent -> <new-id>`; `sessionmgr list` confirmed the source session at
   `SwitchedAway` and the new session at `NeedsInput`, running a real
   `gemini` process whose first prompt was the rendered handoff of the
   Claude Code session's own transcript — Phase 7's mechanism, reached
   from the TUI for the first time.
3. Both runs used a freshly isolated `SESSIONMGR_HOME` (a scratch
   directory, not the real per-user state root) and were torn down
   (`sessionmgr close --discard` on every session, `daemon shutdown`,
   directory removed) immediately after — no state or process left
   behind on the machine this ran on.

### Tests

2 new unit tests (`app::tests::parse_agent_name_accepts_every_known_agent`,
`parse_agent_name_rejects_an_unknown_name`) for the one new piece of pure
logic this update added. No new black-box subprocess tests for the
palette itself — same reasoning `docs/phase-4-report.md` and
`docs/phase-4b-report.md` already gave for the palette's original build:
driving a real terminal UI through `tests/common`'s pattern would mean
scripting `crossterm` input against a `TestBackend`, and the live
verification above is this update's acceptance evidence instead.

`cargo build`/`clippy --workspace --all-targets -- -D warnings`/
`fmt --all --check` all clean. `cargo test --workspace` green aside from
the same pre-existing, undiffed `agent_needs_input_claude`
interactive-PTY flake every phase report since 5 has documented
(confirmed unrelated: reproduces identically on unmodified `main`).

### What is still not done

Lineage grouping in the grid layout (visually clustering a forked
session next to the session it came from) remains unbuilt — issue #22
scoped palette actions only, not layout. A graphical/desktop front end
is separately tracked as issue #23.
