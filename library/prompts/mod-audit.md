---
name: mod-audit
description: Audit recent Claude Code sessions and propose five mods worth building.
tags: [claude-code, mods, workflow]
source: original; idea from Chase AI, "Claude Mods Is The Biggest Claude Code Upgrade Since Skills" (YouTube Rn4nmFRPe0s)
---
Analyze my last 30 Claude Code sessions. Find repeated friction: commands I
retype, mistakes I correct, context I re-supply, risky actions I undo.

Propose exactly 5 mods (hook-based plugins). For each give:
- **Name** and one-line purpose
- **Evidence**: which sessions showed the need
- **Hook**: the event(s) it uses (`tool.call`, `prompt.submit`, `ui.render`, ...)
- **Risk**: what it could break; mods run in-process and unsandboxed

Rank by value over effort. Do not build anything until I pick one.
