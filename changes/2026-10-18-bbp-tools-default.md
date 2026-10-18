---
category: Fixed
changelog: **BBP evidence launchers.** `--tools default` is refused: it means every tool, the Agent tool included, so it let a subagent start on an unpinned model.
---
## 2026-10-18 - BBP evidence launchers: `--tools default`

- **Fixed:** the subagent check accepted any `--tools` list that did not name `Agent` or `Task`, including `--tools default`, which `claude --help` defines as all tools. Captured against Claude Code 2.1.301: with `--tools default` the harness started a subagent. A `--tools` list that names `default` (any case) is now refused; `--tools ""` (no tools) and explicit lists without `Agent` and `Task` still pass.
- **Tests:** `a_tools_list_that_says_default_allows_the_agent_tool` (red before the fix) and the capture `tools-default-starts-a-subagent.jsonl`.
- Host only: no engine, schema or log path.
