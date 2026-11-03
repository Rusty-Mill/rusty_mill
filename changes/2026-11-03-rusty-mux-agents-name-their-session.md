---
category: Added
changelog: rusty_mux `agent setup --name-session`: a coding agent names a session that still has a generated name after its task.
---
## 2026-11-03 - rusty_mux agents name their session (C50, part 2)

- **Added:** `rusty-mux-ctl agent setup --name-session` (opt-in, for Claude Code, Codex, OpenCode and Pi) installs the session-start hook as `agent hook --name-session`. When the agent starts in a session whose name the server made up, such as `quiet-harbor`, and nobody has renamed since, the hook adds one sentence asking it to name the session after its task with `rename-session`. A name someone chose is left alone, even one of the generated shape.
- **Added:** the server remembers which session names it made up: `Session::generated` in `rusty_mux_core`, cleared by a rename, and the `list` session entry's `generated` (field 51, sent only when set, skipped by older decoders).
- **Changed:** `agent status` and `agent setup` take `--name-session` too: status checks for that form of the hook, and setup without the flag puts the plain hook back. Hooks of either form count as setup's, so `agent remove` removes both.
- **Known limitations:** the hook asks the server for the name when the agent starts, waiting at most 2 s. An agent that starts outside a block, or against a server that does not answer, gets no naming sentence. Untested with live agents and on Windows.
