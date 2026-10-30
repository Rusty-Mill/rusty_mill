---
category: Added
changelog: Add a `mux-expert` Claude Code subagent that reviews rusty_mux remote-session and agent-contract changes.
---
## 2026-10-30 - mux-expert review agent

- **Added:** `.claude/agents/mux-expert.md`, a read-and-run reviewer for rusty_mux diffs. It judges lossy-link, stream-fidelity, agent-contract and secrets invariants against the mux docs, earlier review fixes, mosh and tmux control mode, and reports P0–P2 findings with traces and regression tests.
- **Known limitations:** it reviews only; it does not fix code or post to GitHub.
