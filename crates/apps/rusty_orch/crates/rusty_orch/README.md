# rusty_orch (binary)

The orchestrator as a command. One JSON goal file in; the plan, the live board, and the call ledger out.

```sh
cargo run -p rusty_orch -- run crates/apps/rusty_orch/crates/rusty_orch/examples/goal.json
cargo run -p rusty_orch -- run goal.json --interactive --json
cargo run -p rusty_orch -- --help
```

| Flag | Meaning |
|---|---|
| `--ollama-model <name>` | model for `Agent::Local`; env `ORCH_OLLAMA_MODEL`; default `llama3.2` |
| `--codex-repo <dir>` | repository Codex may read; env `ORCH_CODEX_REPO`; default: current directory |
| `--codex-model <name>` | model for `Agent::Codex`; default: Codex's own |
| `--interactive` | when a card blocks on a question, read the answer from stdin and keep going |
| `--json` | print the report as one JSON object instead of text |

Exit status: `0` finished, `3` blocked on unanswered questions, `4` budget or agent failure, `2` usage, `1` any other error.

Channels: stdout carries the report and nothing else, so `--json` is always one parseable object even through a blocked-and-answered run. Questions, the answer prompt, and progress lines go to stderr; answers are read from stdin. A progress line is built only from a fixed outcome category, task ids, agent names, and counts; adapter or model error text appears only in the report. The questions shown under `--interactive` are model text by nature. A blank line or end of input stops the question round at once: answers already given stay on the board, no further model call is made, and the run exits `3`.

## Goal file

The goal contract fields of `orch_core::goal::GoalDraft` by name (`goal`, `done_when`, `in_scope`, `out_of_scope`, `constraints`, `refs`, `wall_clock_secs`, `max_calls`, `stop`), a `tasks` array, and an optional `routing` object. `depends_on` and a review's `target` are indices into `tasks` and must point at an earlier task. Refs use the adapter protocol's syntax: `path:`, `commit:`, `url:`, `E-<n>`. See [`examples/goal.json`](./examples/goal.json) and [ADR-0009](../../docs/adr/0009-command-line.md).

Routing defaults: Codex for research, design, and implement; the local model for triage; reviewers `local` then `codex`. `routing`, when present, must be an object; anything else is rejected. Neither adapter serves `implement` yet, so such a card fails at once (ADR-0007).

A review card needs no `refs` to see what it reviews: the shared renderer resolves the reviewed task's live entries, and the answers to them, at render time (`UNDER REVIEW`), so the author never has to guess entry ids.

## Tests

```sh
cargo test -p rusty_orch
```

Unit tests cover the argument parser. Integration tests drive the goal-file parser through every rejection and the run loop through finished, blocked, answered, blank answer, wall clock, and dispatcher failure, over `orch_dispatch::fake::FakeAgent`. No real binary is run.
