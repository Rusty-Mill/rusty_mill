# Phase 3 report — Claude Code and Codex adapters

Phase 3's stated scope (PLAN.md § Phased milestones): "Codex and Gemini
adapters." Scoped down to **Claude Code and Codex** for this pass — Gemini
CLI has a confirmed hook mechanism (`gemini hooks`) but nothing about it has
been verified against a real, running session on this machine (no
credentials). Claude Code's own adapter did not exist as code before this
pass either (`sessionmgr-agents` did not exist), despite Phase 1/2 running
`claude` sessions through the generic command path — building it now, first,
is what the other two adapters are patterned on.

## Status

| Item | Outcome |
|---|---|
| `AgentAdapterPort` trait, declared | **Done** |
| `sessionmgr-agents` crate: `ClaudeCode`, `Codex` | **Done**, evidence-based |
| Tier-2 (process exit) | **Already existed** (`Session::record_exit`), reused as-is |
| Tier-3 (pattern matching) | **Done**, real captured patterns, live-verified |
| Tier-1 (hooks) | **Not wired** — deliberately, see below |
| Gemini adapter | **Not built** — credentials-gated, see below |
| Live verification | **Done** — two gated black-box tests pass against real CLIs |

## What tier-3 needed, and the discovery that changed the design

The obvious approach — strip ANSI escapes from raw output with a regex, then
substring-match — was tried first and **measured to silently fail**. Both
Claude Code and Codex lay out large parts of their screen using
cursor-positioning escape sequences rather than literal space characters.
Stripping the escapes without interpreting them collapses `"Welcome back
Nano!"` into `"WelcomebackNano!"` — every pattern spanning a word boundary
breaks, silently, with no error.

The fix: run recent output through a real `vt100` screen (already a
workspace dependency — `sessionmgr-tui` uses it for the same ADR-0002 reason)
and match against `Screen::contents()`'s rendered text, never raw bytes. This
is now `sessionmgr-agents::pattern_watch::ScreenWatcher`, fed inline from
`worker.rs`'s existing PTY-reader thread and piped-stdio pump — no new
concurrency, since `vt100::Parser::process` is pure computation, not I/O.

## Real captures, not assumed patterns

Both adapters' pattern sets come from actually running `claude` and `codex`
through `sessionmgr` on this machine and rendering the transcript through
`vt100` (via `pyte`, Python's `vt100` implementation, for the investigation
pass — the shipped code uses the real `vt100` crate). Full transcripts are
not reproduced here (they're long and machine-specific); the durable
substrings extracted from them are checked into `claude_code.rs`/`codex.rs`'s
own test fixtures verbatim.

**Claude Code**: the bottom status bar is the tell. `⏸ manual mode on · esc
to interrupt · ← for agents` while actively working; `⏸ manual mode on · ?
for shortcuts · ← for agents` once idle again. Checked first and
unconditionally, `"esc to interrupt"` means `Running` regardless of anything
else on screen — this is what stops a tool-call's own `"Waiting…"` line from
being misread as the session waiting on the *user*. Everything else
(`"Do you want to proceed?"`, `"requires approval"`, `"Quick safety check"`,
`"? for shortcuts"`) means `NeedsInput`.

**Codex**: no equivalent persistent status bar was found, but two genuinely
different dialogs (the folder-trust gate, and a plugin-hooks review screen
triggered by marketplace-installed hooks) share one consistent phrasing:
`"Press <key> to <verb>"`. That shared shape, not a per-dialog string list,
is the durable pattern.

## The Codex hook + sandbox discovery

Verifying Codex's hooks fire on Windows (this pass's other real research
question) surfaced a genuine, previously undocumented interaction: **a hook
command runs under the same `--sandbox` policy as the agent's own tool
calls.** Configured `SessionStart`/`Stop` hooks in `.codex/config.toml`,
each writing a marker file, silently failed (`hook: SessionStart Failed`, no
file written) under Codex's default `read-only` sandbox — writing a file is
exactly what `read-only` forbids. Re-run with `--sandbox
danger-full-access`, both hooks fired and both files existed.

This is not "Codex hooks don't work on Windows" (they do — verified,
`has_verified_hooks() -> true`). It is a real constraint on *installing*
them: a hook that needs to reach sessionmgr's own daemon socket (Phase 4's
`__hook-fire`) needs a sandbox mode that permits it, and the failure mode
without that is silent, not a loud error. Recorded here for Phase 4's hook-
install work to account for, not solved by this pass.

## Why tier-1 (hooks) is not wired, on purpose

PLAN.md's own section header is explicit: "Session hooks / extensibility
(**Phase 4+, not earlier**)." Installing hook config into a CLI's own
settings file (`.claude/settings.json`, `.codex/config.toml`) and standing up
`sessionmgr __hook-fire` to receive the callback is real, separately-scoped
work — and per PLAN.md, security-sensitive enough to deserve its own pass
(secret-scrubbing, the `__hook-fire`-must-no-op-on-an-unrecognized-session-id
requirement, since a globally-installed hook fires for *every* session on the
machine, not just this tool's own).

So `AgentAdapterPort::has_verified_hooks()` exists and is `true` for both
adapters — recording that the mechanism is proven, for Phase 4 to build on —
but nothing in this pass ever writes to a CLI's own config file. Tier-3 is
the only detection signal a session actually gets today, exactly as tiered:
tier-2 (process exit) is free and always-on regardless of adapter; tier-3 is
real and live; tier-1 is verified-but-not-connected.

## Why Gemini is not here

`gemini hooks migrate --from-claude` is real, strong static evidence that
Gemini's hook mechanism exists and is deliberately Claude-Code-compatible.
It was not tested against a real session: this machine's `gemini` install has
no credentials (`GEMINI_API_KEY`/Vertex/GCA all unset, no `gcloud` session to
borrow), and `SessionStart` never fires before the CLI's own auth check
fails first. Building a `Gemini` adapter now would mean shipping pattern
data nobody has actually seen. `AgentKind` and `sessionmgr-agents::lib.rs`'s
docs both say explicitly this is a one-file, one-variant addition once
credentials exist — not a redesign later.

## `--agent` and what it changes

`sessionmgr new --agent claude|codex [--kind ...] [-- <prompt>]`:

- `command` (whatever followed `--`) is resolved through the adapter's own
  `launch_args`, not treated as the literal program to run — empty means
  bare interactive, non-empty is passed through as an initial prompt.
- The session's `agent` field is recorded (`state.json`, and surfaced on
  `SessionSummary` for a future TUI confidence badge), turning on tier-3
  detection for the life of the session.
- Without `--agent`, behavior is byte-for-byte what it always was: `command`
  runs literally, only tier-2 (exit code) is ever reported. Fully backward
  compatible — every existing `state.json` still loads (`agent` is
  `#[serde(default)]`), every existing call site of `Session::new` and
  `client::session_new` was updated, none of their behavior changed.

## Live verification

Two new gated black-box tests
(`tests/agent_needs_input_{claude,codex}.rs`), skipped cleanly (not failed)
when the CLI isn't on `PATH`, run for real here since both are installed:

```
test a_fresh_claude_session_reaches_needs_input_on_its_own ... ok  (1.80s)
test a_fresh_codex_session_reaches_needs_input_on_its_own ... ok   (1.46s)
```

Each creates a real worktree session with `--agent <kind>` against a
throwaway repository, sends **no input at all**, and asserts the session
reaches `needs-input` on its own within 60 seconds — proving the whole
pipeline (daemon → detached worker → real PTY → real CLI → `vt100` →
adapter → `Session::transition_to` → `state.json`) against the CLI's own
first-run folder-trust prompt, not a synthetic fixture.

## Tests

15 new unit tests in `sessionmgr-agents` (both adapters' `launch_args` and
`needs_input`, against the real captured screen text above, plus
`ScreenWatcher`'s own cursor-positioning regression test), 2 new gated
black-box tests. `cargo build`/`clippy --all-targets -D warnings`/`fmt
--all --check`/`test --workspace` all green on real Windows.
