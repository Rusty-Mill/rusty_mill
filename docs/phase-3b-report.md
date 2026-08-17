# Phase 3b report — the Gemini CLI adapter

`docs/phase-3-report.md` scoped Phase 3 down to Claude Code and Codex,
explicitly deferring Gemini: "this machine's `gemini` install has no
credentials... Building a `Gemini` adapter now would mean shipping
pattern data nobody has actually seen." Its own closing line: "`AgentKind`
and `sessionmgr-agents::lib.rs`'s docs both say explicitly this is a
one-file, one-variant addition once credentials exist — not a redesign
later."

Gemini CLI (`@google/gemini-cli@0.55.1`) turned up installed on this
machine. Credentials still do not exist here — no `GEMINI_API_KEY`, no
Vertex/GCA env vars, no `~/.gemini/settings.json` auth config. This
report is honest about exactly what that does and does not change.

## Status

| Item | Outcome |
|---|---|
| `AgentKind::Gemini` + `gemini.rs` adapter | **Done** |
| Hook mechanism's existence, config format, event list | **Confirmed** — primary-source docs, not guessed |
| Tier-3 `needs_input` patterns | **Source-derived**, not live-captured — see below |
| Tier-1 hooks (`has_verified_hooks`) | **Honestly `false`** — mechanism confirmed, firing not observed |
| `--agent gemini` CLI wiring | **Done** |
| Daemon-side plumbing (session creation, hook-config install, tier-2 exit) | **Live-verified** against a real `gemini` process |
| Hook firing against a live gemini-backed session | **Not verified** — blocked, see below |
| Gated regression test (`agent_needs_input_gemini.rs`) | **Added**, skips cleanly here, proves itself the day credentials exist |

## Why this is not the same rigor as Claude Code/Codex, and why it is still real

Claude Code's and Codex's patterns were captured by literally running each
CLI through `sessionmgr` on this machine and reading the rendered
`vt100` screen. That path is closed for Gemini: `gemini` hard-refuses to
start — before reaching any interactive screen, before firing a single
hook — without an auth method configured. Verified directly: a
`SessionStart` hook installed in a scratch repo never fired when `gemini`
ran there unauthenticated (the auth check happens first, unconditionally).

What was available instead, and used: `gemini-cli`'s own **shipped
source**. The npm package's `bundle/*.js` files are minified (identifiers
mangled) but not string-literal-obfuscated — the exact text the UI
renders is sitting in there verbatim. Grepping it directly for dialog and
status-bar text is, if anything, *more* precise than transcribing a live
terminal capture: it is the literal constant, not a transcription of one
observed render. What it is not: proof that the constant actually reaches
the screen the way the code around it implies, under real runtime
conditions this project doesn't control (terminal width, Ink's own
render timing, etc.) — which is exactly what live capture would have
added and doesn't get to add here.

Extracted, with file:line provenance kept in `gemini.rs`'s own doc
comments:

- **Busy indicator**: `"esc to cancel"` (`interactiveCli-*.js`, rendered
  in `cancelAndTimerContent` exactly while `streamingState ===
  "responding"`). Different phrase from Claude Code's `"esc to
  interrupt"` — the two CLIs do not share this one.
- **Workspace-trust gate**: `"Do you trust the files in this folder?"`
  (the trust dialog's own title, `RadioButtonSelect` component).
- **Tool confirmation**: `"Do you want to proceed?"` (the `"info"`-type
  confirmation question) and `"Allow execution of"` (the general and
  MCP-tool confirmation question prefix — both variants share it).
- **Auth-blocked state**: `"You must select an auth method to proceed"`.
- **Idle marker**: `"? for shortcuts"` — textually identical to Claude
  Code's own idle marker. Not investigated further whether that is
  coincidence or shared UI-library convention; it is a real extracted
  string either way, from Gemini's own source, independent of Claude
  Code's.

Hooks: the official reference
(<https://geminicli.com/docs/hooks/reference/>) names the full event
list, config schema, and communication protocol precisely — `settings.json`
under `.gemini/`, `matcher` as an exact string for lifecycle events like
`SessionStart`'s `"startup"`, stdin/stdout JSON, "silence is mandatory"
(a script must print only JSON to stdout — `sessionmgr __hook-fire`
already prints nothing on success, so this is satisfied trivially, no
adapter-side change needed). `gemini hooks migrate --from-claude`
existing at all is corroborating evidence the mechanism is real and
deliberately Claude-Code-compatible in shape.

**No Gemini analog to `SubagentStop` exists** in the documented event
list — confirmed by reading the full reference, not by omission. This
adapter's `hook_signal` has no `HookOutcome::Notify` mapping as a result,
a real and stated difference from Claude Code/Codex, not an oversight.

## `launch_args` — one flag, not two

`--skip-trust` (confirmed via `gemini --help`: "Trust the current
workspace for this session") is added when `--hooks` is requested,
mirroring Claude Code's simpler one-flag model rather than Codex's
two-flag one (`--dangerously-bypass-hook-trust` *and* `--sandbox
danger-full-access`). Source inspection found no separate hook-trust-
review gate or hook-specific sandbox flag — the hooks docs' own
"Project-level hooks are particularly risky when opening untrusted
projects" reads as hooks being gated by the *same* trust mechanism as
everything else, not a second one. Flagged in the adapter's own comments
as inferred, not measured — this is exactly the kind of Codex-specific
surprise (a hook silently no-op'd by a sandbox policy) that a wrong guess
here would reproduce silently, so it is called out rather than asserted.

## Live verification — what actually ran, and what didn't

**What ran, for real, against the actual installed `gemini` binary and a
real daemon on this machine:**

1. `sessionmgr new --kind worktree --agent gemini --hooks --repo <real
   repo>` wrote `.gemini/settings.json` with the exact expected shape —
   `SessionStart`/`Notification`/`AfterAgent` groups, the right
   `__hook-fire --session-id <id> --event <name>` command per event
   (forward-slashed binary path, correct session id), and `"matcher":
   "startup"` present only on `SessionStart`.
2. The real command line launched was `gemini --skip-trust ...` — the
   flag construction confirmed against the actual spawned process, not
   just the unit test.
3. `gemini` itself failed on its own auth check
   (`"Please set an Auth method..."`, exit code 41) — and tier-2 (process
   exit, already-existing, agent-agnostic code) correctly recorded the
   session `Errored`. This is the real failure mode this environment
   produces; it is not being hidden or worked around.
4. `sessionmgr __hook-fire` against that now-`Errored` session was
   correctly a no-op (`expects_live_worker()` is false past a terminal
   status) — the same generic safety check already proven for Claude
   Code and Codex, exercised again here by the natural consequence of
   the auth failure rather than by design.

**What did not run, and could not, on this machine:** firing a hook
against a *live* gemini-backed session. `--agent gemini` always launches
the real `gemini` binary (same as every adapter — the adapter decides the
literal program, not the caller), and `gemini` cannot stay alive past its
own auth check without credentials. There is no way to construct a
long-lived gemini-backed session here to fire a hook against. This is not
new daemon-side surface, though: `Supervisor::hook_fire` and
`Worker::handle_hook_event` are entirely adapter-agnostic (`adapter_for`
is the only place `AgentKind` is matched in that path), and both were
already proven correct against two other real, live adapters in
`docs/phase-4b-report.md`. What's new and specific to Gemini —
`hook_signal`'s event mapping, `hook_config`'s output shape, `needs_input`'s
patterns — is covered by 14 new unit tests instead, plus a gated
black-box test (`agent_needs_input_gemini.rs`, modeled on the existing
`claude`/`codex` ones) that skips cleanly today and will run for real the
day this machine — or a CI runner — has Gemini credentials.

## Tests

14 new unit tests in `sessionmgr-agents::gemini` (`launch_args`,
`needs_input` against every extracted marker plus the busy-wins-first
case, `hook_config`'s exact output shape including the `SessionStart`-only
matcher, `hook_signal`'s mapping and its deliberate absence of `Notify`).
One new gated black-box test, skipping cleanly here. `cargo
build`/`clippy --workspace --all-targets -- -D warnings`/`fmt --all
--check`/`test --workspace` all green on real Windows.

## What would change this report's confidence

Getting `GEMINI_API_KEY` (or another supported auth method) configured on
a machine that also has `sessionmgr` built, then: running
`agent_needs_input_gemini.rs` for real (proves or disproves every
`needs_input` marker at once), and manually firing each hook event
against a live session the way `docs/phase-4b-report.md` did for Claude
Code and Codex (proves `hook_signal`'s mapping and flips
`has_verified_hooks` to `true` honestly, not preemptively).
