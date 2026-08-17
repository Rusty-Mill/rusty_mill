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
| Gemini CLI adapter | **Deferred** — filed as [#15](https://github.com/baileyrd/rusty_yirp/issues/15) |
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

## Why Codex and Gemini CLI are deferred, not guessed at

Both have real fork mechanisms — ADR-0003 already established that.
Both are missing one specific, identified piece, not "everything":

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

**Codex and Gemini CLI Fork support** — filed as issues, not silently
dropped; see above.

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
