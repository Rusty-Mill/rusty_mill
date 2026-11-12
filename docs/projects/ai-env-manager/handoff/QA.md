# Implementer Q&A — ai-env-manager
Questions an implementer would ask, answered from the sources. Each cites its source.
`ai-env-manager-expert` adds more as they are asked.

## Scope
- **Q:** Is this only a skills manager? **A:** No. Skills, plugins/mods, agents, prompts and key md files (D2, D10). M0 still ships skills only. [stated]/[default]
- **Q:** Can I start a crate or file cards? **A:** Not until Nano sets a priority (D13). [default]
- **Q:** Is there a built-in editor? **A:** No; "open in editor" is enough. Source § From Skills Manager. [default]
- **Q:** Usage stats? **A:** Later, opt-in only, never default-on hooks. Source § From Skills Manager. [default]

## Linking
- **Q:** Link whole tool folders or single entries? **A:** Single entries, per tool (D6). [default]
- **Q:** Symlinks fail on Windows without Developer Mode. **A:** Fall back to a junction (no admin needed), then a marked copy (D6). [default]
- **Q:** May I delete a folder in a tool's skills dir that we didn't create? **A:** Never. Only marker-carrying paths; otherwise refuse and explain (D6). [default]
- **Q:** Some agents read `~/.claude/skills` natively. **A:** Record a `native` link mode per tool so no duplicate link is made; verify each claim before trusting it. Source § From Skills Desktop. [default]

## Scanner
- **Q:** Which rules? **A:** SM's context-aware parser plus SD's broader categories (secrets, sensitive files, command injection, Windows destructive commands), rewritten by us as TOML data; bundled-binary check; 0–100 score with hard-block rules; SARIF output (D7, D9). [default]
- **Q:** Report only, or gate? **A:** Gate: install scans first and stops on a hard block. Source § From Skills Desktop. [default]

## Data and deps
- **Q:** Where is the tool list? **A:** A TOML catalog; custom tools are entries in the same file (D7). [default]
- **Q:** reqwest/tokio? **A:** House crates first; sync until needed; new third-party deps are ask-first (D12). [default]
