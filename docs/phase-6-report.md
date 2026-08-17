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
| Codex adapter | **Deferred** — filed as [#14](https://github.com/baileyrd/rusty_yirp/issues/14) |
| Gemini CLI adapter | **Done**, 2026-08-17 — see this report's own dated update below |
| Live verification | **Done** — real mechanism proven, plus an honest limit on what could be proven in this environment |

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

## Why Codex was deferred, not guessed at (Gemini CLI closed this gap 2026-08-17 -- see below)

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

**Codex Fork support** — filed as issue #14, not silently dropped; see
above and this report's own 2026-08-17 update. (Gemini CLI Fork support
shipped as of that update.)

**Switch-agent-mid-session** — entirely untouched by this phase.
ADR-0003 scoped the format each CLI expects, which is the real
prerequisite for translating one CLI's transcript into another's, but
no translation code exists yet. This remains real, separately-scoped
work, not started here.

**TUI support for Fork.** `SessionSummary.forked_from` is surfaced over
the wire and in `sessionmgr list`'s new `FORKED-FROM` column, matching
Phase 5's own precedent for `parent` — enough for a future TUI pass to
build a "Fork" command-palette action and lineage grouping on top of,
but no TUI-side UI was built in this pass.

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

**Codex Fork (issue #14)** remains open. Unlike Gemini's file-location
problem, Codex's own blocker is a genuinely separate piece of
machinery -- a post-spawn filesystem watch to discover its
self-assigned thread id, wired into `Supervisor`/`worker.rs` rather than
a pure adapter method -- plus this session's own billing-quota block
(see `docs/phase-7-report.md`) still prevents live-verifying
content-preservation even if that machinery existed. Left for its own
dedicated PR rather than a rushed addition here; issue #14's own
2026-08-17 comment has the current state of what is and is not known.
