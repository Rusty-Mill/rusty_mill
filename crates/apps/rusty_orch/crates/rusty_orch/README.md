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
| `--repo <dir>` | repository Codex and Claude read, as their working directory; env `ORCH_REPO`; default: current directory. `--codex-repo` and `ORCH_CODEX_REPO` are aliases |
| `--codex-model <name>` | model for `Agent::Codex`; default: Codex's own |
| `--claude-model <name>` | model for `Agent::Claude`; default: Claude Code's own |
| `--interactive` | when a card blocks on a question, read the answer from stdin and keep going |
| `--json` | print the report as one JSON object instead of text |
| `--state <dir>` | save the plan, board, and ledger there after every dispatcher run and answer round, and resume from it on the next run; env `RUSTY_ORCH_STATE`; default: in memory only |

Exit status: `0` finished, `3` blocked on unanswered questions, `4` budget or agent failure, `2` usage, `1` any other error.

Channels: stdout carries the report and nothing else, so `--json` is always one parseable object even through a blocked-and-answered run. Questions, the answer prompt, and progress lines go to stderr; answers are read from stdin. A progress line is built only from a fixed outcome category, task ids, agent names, and counts; adapter or model error text appears only in the report. The questions shown under `--interactive` are model text by nature. A blank line or end of input stops the question round at once: answers already given stay on the board, no further model call is made, and the run exits `3`.

## Resuming

`--state <dir>` keeps the goal's plan, board, and ledger in that directory ([ADR-0010](../../docs/adr/0010-persistence-on-multimodal-db.md)). A run that exits `3` can be continued later with the same goal file and directory, typically with `--interactive` to answer what blocked it. Calls recorded in the recovered checkpoint count against the resumed ceilings. Abrupt termination can repeat work and calls since the last successful checkpoint; those lost charges are not reconstructed. The goal file is fingerprinted: a different file against saved state is refused with exit `1`, so use a fresh directory per goal. One process holds a directory at a time. The wall clock is per invocation. Because persistence is unconditional in the binary, its Rust requirement is 1.89.

```sh
rusty_orch run goal.json --state .orch            # exit 3: a card asked a question
rusty_orch run goal.json --state .orch --interactive   # answer, continue, finish
```

## Goal file

The goal contract fields of `orch_core::goal::GoalDraft` by name (`goal`, `done_when`, `in_scope`, `out_of_scope`, `constraints`, `refs`, `wall_clock_secs`, `max_calls`, `stop`), a `tasks` array, and an optional `routing` object. `depends_on` and a review's `target` are indices into `tasks` and must point at an earlier task. Refs use the adapter protocol's syntax: `path:`, `commit:`, `url:`, `E-<n>`. See [`examples/goal.json`](./examples/goal.json) and [ADR-0009](../../docs/adr/0009-command-line.md).

`stop` is `checkpoint` or `best_effort`. Under `checkpoint` a card that asks a question blocks until a human answers (exit `3`, or `--interactive`). Under `best_effort` the adapters withdraw the `question` kind and tell the agent to write an `assumption` entry stating what it takes as true and continue; the run ends without a human and the text report lists every assumption in its own block ([ADR-0011](../../docs/adr/0011-best-effort-stop-rule.md)).

Routing defaults: Codex for research, design, and implement; the local model for triage; reviewers `local` then `codex`. Claude is reached by naming `claude` in `routing` ([ADR-0012](../../docs/adr/0012-claude-adapter.md)); `gemini` has no adapter yet. `routing`, when present, must be an object; anything else is rejected. No adapter serves `implement` yet, so such a card fails at once (ADR-0007).

A review card needs no `refs` to see what it reviews: the shared renderer resolves the reviewed task's live entries, and the answers to them, at render time (`UNDER REVIEW`), so the author never has to guess entry ids.

## Tests

```sh
cargo test -p rusty_orch
```

Unit tests cover the argument parser. Integration tests drive the goal-file parser through every rejection, the run loop through finished, blocked, answered, blank answer, wall clock, dispatcher failure, and a partial answer round whose second read fails. Repeated entry-point calls in one test process cover resume logic over `orch_dispatch::fake::FakeAgent`; a separate subprocess test executes the built binary twice with a scripted local CLI and reopens the state directory. The deterministic store fault test exercises error/drop/reopen recovery, not an OS process crash.
