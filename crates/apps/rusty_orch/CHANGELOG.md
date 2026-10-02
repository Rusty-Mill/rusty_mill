# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Fixed
- `orch-ollama` real-binary test asserts a per-run nonce appears in an entry body instead of merely a non-empty reply, matching `orch-codex` (#441).
- `orch-cli` `render`: a card's own live entries and the answers to them are inlined, so a card resumed after a `Question` sees its `Answer` (#448).
### Added
- `orch-cli` crate: the CLI-adapter core extracted from `orch-ollama` (process seam, JSON reply parser, prompt core with a per-adapter footer, `fake` test doubles). `CommandRunner::run_scrubbed` removes named variables from the child environment. ADR-0004 amended.
- `orch-codex` crate: `Agent::Codex` over `codex exec --sandbox read-only --ephemeral --ignore-user-config --ignore-rules -C <root> --output-schema --output-last-message -`, strict `anyOf` output schema, prompt on stdin, `OPENAI_API_KEY` scrubbed, `not logged in` and `rate limited` told apart by stderr. `research_review` example (Codex researches, Local reviews). ADR-0006.
- `orch-cli`: the `review` kind for `Role::Review` cards (`verdict`: approve or changes_requested; exactly one per reply). Shared `excerpt` helper.
### Changed
- `orch-ollama` is a thin adapter on `orch-cli`; behaviour and tests unchanged.
- `orch-core`: an agent-authored `Decision` must cite a live approving `Review` by a different author; self-approval is rejected with the existing `DecisionNeedsApproval`. ADR-0005 updated.
- `orch-ollama`: `StdCommand` spawns the child as a process-group leader and kills the whole group (unix `kill -KILL -- -<pgid>`, Windows `taskkill /T /F`) on timeout or overflow; pipe threads are joined for at most `JOIN_GRACE` (2s) and detached otherwise, so the deadline is a hard bound. ADR-0004 updated.
- `orch-core`: `Board::append` refuses an agent-authored `Decision` unless a ref points at a live approving `Review`; new `BoardError::DecisionNeedsApproval`. Applies to supersessions. ADR-0005.
### Changed
- `orch-ollama`: `decision` removed from every role's allowed kinds; the prompt spec tells the model to propose decisions as findings.
- `orch-ollama` crate: `Agent::Local` over the Ollama CLI. `render` (card + referenced live entries + format spec), strict `parse` (kinds per role, confidence, ref syntax, caps), `OllamaAgent` over a `CommandRunner` seam with `StdCommand` (stdin thread, deadline, kill and reap, 1 MiB stdout cap). `research` example and an ignored real-ollama test. ADR-0004 (output protocol).
- `orch-dispatch` crate: `AgentRunner` port, validated `Routing` (work roles → one agent; reviews → ordered preference list, first agent ≠ author), sequential `Dispatcher` loop over `Plan` with goal and per-task call ceilings in a caller-owned `Ledger`, atomic per-call board writes, retry exhaustion → `Plan::fail`, `FakeAgent` behind the `fake` feature. ADR-0003 (dispatcher reuse boundary).
- `orch-core` domain crate: goal contracts, task cards with `Plan` lifecycle, append-only blackboard.
- Repo governance set, Rust CI, `AGENTS.md`/`CLAUDE.md`, ADR-0001 and ADR-0002.
### Fixed
- `SECURITY.md` advisory link points at `Rusty-Mill/rusty_mill` instead of the retired standalone repo.
