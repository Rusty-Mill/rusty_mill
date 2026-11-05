---
category: Added
changelog: rusty_mux shows Claude Code's and Codex's status from their own hooks (a turn, a tool, a permission prompt), set up by `agent setup`.
---
## 2026-11-05 - rusty_mux agent status hooks for Claude Code and Codex (C74, O2 part 2)

- **Added:** `agent setup` adds a status hook per event to Claude Code's `settings.json` and
  Codex's `hooks.json`, beside the session-start hook: a turn is `working`, a permission prompt
  `blocked` with the tool's name, a finished turn `idle`, and for Claude Code a failed turn `error`.
- **Added:** `rusty-mux-ctl agent hook --event STATE --tool-from-stdin` takes the tool's name from
  the hook event's JSON on stdin; nothing else from it is sent.
- **Changed:** an install from an older ctl shows `outdated` until `agent setup` runs again. Remove
  with the newer ctl: an older one removes only the session-start hook.
- **Known limitations:** event names come from the Claude Code and Codex hook references, not a
  run against the real agents. Codex runs a new hook only once it is trusted (`/hooks`).
