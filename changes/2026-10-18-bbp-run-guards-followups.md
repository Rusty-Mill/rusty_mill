---
category: Fixed
changelog: **BBP run guards, follow-ups to #763.** `--tools default` and a checked flag given twice are refused in evidence launchers, `min_commit` in the criteria pins the #763 merge commit, and the stop kind, the criterion 1 scope and the "no result" reason are covered by tests.
---
## 2026-10-18 - BBP run guards: follow-ups to #763

- **Fixed:** the subagent check accepted `--tools default`, which `claude --help` defines as every tool. Captured against Claude Code 2.1.301: with `--tools default` the harness started a subagent (`tools-default-starts-a-subagent.jsonl`). A `--tools` list that names `default` (any case) is now refused; `--tools ""` and explicit lists without `Agent` and `Task` still pass.
- **Fixed:** a checked flag (`--tools`, `--disallowedTools`, `--output-format`, `--setting-sources`) given more than once is refused, since the harness may read the last or a merge of the two where the check read the first.
- **Fixed:** a turn with messages but no result now says `no result line`, not `assistant messages after the last result`; dead code in the cost floor is removed.
- **Changed:** `docs/bbp/2b-criteria.json` pins `min_commit` to `735f1b25e8479424181733756307e0bba8eef200`, the merge commit that contains both the verdict-binding fix (#727) and the run tooling (#763). `prices_as_of` and `status` are unchanged; the criteria and the run remain unapproved.
- **Tests:** a test for each of the third review's surviving mutations: the end-of-run stop kind is `cap`, the `--disallowed-tools` alias, overshoot counted in N=2 runs only, and a message after the last result leaves the turn incomplete.
- Host and docs only: no engine, schema or log path.
