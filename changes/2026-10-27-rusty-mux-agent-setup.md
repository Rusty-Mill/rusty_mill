---
category: Added
changelog: rusty-mux-ctl agent setup, status and remove: install, check and undo a rusty_mux skill and session-start hook for Claude Code.
---
## 2026-10-27 - rusty_mux agent setup for Claude Code (C34)

- **Added:** `rusty-mux-ctl agent setup|status|remove [--dir DIR]` writes the rusty-mux skill (our own text) to `skills/rusty-mux/SKILL.md` and one `SessionStart` hook to `settings.json` in Claude Code's config directory (`$CLAUDE_CONFIG_DIR` or `~/.claude`, or a project's `.claude` with `--dir`). Other settings and hooks are kept; `status` exits 0 only when both are current; `remove` deletes only what setup wrote.
- **Added:** `rusty-mux-ctl agent hook`, which the hook runs: inside a block it tells the agent which block, window and session it is in; elsewhere it prints nothing.
- **Added:** `docs/projects/mux/AGENT-PLACEMENT.md`, how targets are chosen (named, then here, else refused) and where an agent puts its blocks.
- **Known limitations:** Claude Code only. Setup rewrites `settings.json` with sorted keys; `remove` restores the original bytes from `settings.json.rusty-mux.bak` only when nothing else changed in between. A skill or settings file setup did not write, or cannot parse, is refused rather than replaced.
