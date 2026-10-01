# ADR-0004: Ollama adapter: one JSON object on stdout, prompt on stdin

- **Status:** Accepted
- **Date:** 2026-10-01

## Context
The first real `AgentRunner` is `Agent::Local` over the Ollama CLI, running one read-only research card and writing real entries to the `Board`. Two things had to be decided: how the model hands entries back, and how the adapter reaches the process.

Small local models are unreliable at free-form structure. A hand-parsed line format (`FINDING high | body | refs`) has nothing enforcing it, and long bodies break pipe-delimited lines. The Ollama CLI's `--format json` grammar-constrains generation, so the model cannot emit prose, fences, or trailing commentary around the object.

`contract::ProcessRunner` (platform layer, allowed by ADR-0003) captures argv, env, cwd, stdout, stderr and exit status, but `ProcessSpec` has no stdin and the native runner hard-codes `Stdio::null()`; there is no timeout either. The prompt must go on stdin (argv is a fixed vector with no interpolation) and a hung model must not hang the dispatcher.

## Decision
### Output protocol
The model replies with exactly one JSON object:

```json
{"entries":[{"kind":"finding","confidence":"high","body":"...","refs":["E-1","path:src/x.rs"]}]}
```

- `kind` is a function of the card's `Role`: Research, Triage and Design may write `finding`, `question`, `assumption`; Implement and Review are not served by this adapter. `decision` is never offered: agents propose decisions as findings and the board refuses unreviewed ones (ADR-0005; Design briefly had `decision` before that ADR).
- `finding` requires `confidence` in `low`, `medium`, `high`. Other kinds omit it.
- `body` is non-blank and at most 500 characters. Detail belongs behind refs.
- `refs` is an array of strings: `E-<n>` for a board entry, or `path:`, `commit:`, `url:` followed by non-blank text. `parse` checks syntax only; existence of `E-<n>` is left to `Board::append`, which already validates it.
- At most 8 entries, at least 1. Over-cap replies are rejected whole.
- The parser tolerates one ``` fence around the object (older CLIs without `--format`) and nothing else. Unknown kinds, blank bodies, missing confidence, and malformed refs are errors, never guesses.

Parsing uses `rusty_json` (foundation layer) with default features off, so the crate adds no registry dependency and keeps its sovereign posture under root ADR-0002. `serde_json` is a workspace dependency and would have been allowed, but would move the crate to Tier A for no gain.

### Process seam
The adapter keeps its own one-method trait, `CommandRunner::run(argv, stdin, timeout)`, with `StdCommand` over `std::process` and an in-memory fake in tests. `StdCommand` writes stdin on its own thread and closes it, drains stdout and stderr concurrently, polls the child against a deadline, kills and reaps it on timeout or when stdout exceeds 1 MiB, and reports both as typed `ExecError`s. It does not depend on `contract`; it collapses into `contract::ProcessRunner` the day that trait grows stdin and a timeout.

### Shell
`OllamaAgent { model, timeout }` serves `Agent::Local` only. Argv is `ollama run <model> --format json`, prompt on stdin. Non-zero exit, timeout, overflow, spawn failure, and empty stdout are `AgentError`s carrying a bounded, single-line excerpt of stderr, never the full prompt or output.

## Consequences
- Entries a small model writes pass the same board validation as any other author, because the dispatcher appends them.
- The protocol is stable text; a second CLI adapter can reuse `parse` as-is and `render` with a different spec footer. A generic CLI-adapter abstraction waits for that second call site.
- The `--format json` and stdin behaviour are asserted by an ignored integration test against the real binary, documented in the crate README, since CI has no model.
- Follow-ups: extend `contract::ProcessSpec` with stdin and timeout, then retire `CommandRunner`; serve Implement and Review once a repo-writing adapter exists.
