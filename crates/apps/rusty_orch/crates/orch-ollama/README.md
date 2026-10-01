# orch-ollama

`Agent::Local` for rusty_orch, over the Ollama CLI. Three pure functions and one thin I/O shell:

| Piece | What |
| ---- | ---- |
| `render(task, board)` | The prompt: instruction, acceptance, refs, the live entries the card's refs point at, then the output-format spec for the card's role. |
| `parse(stdout, role)` | Strict: one JSON object, allowed kinds per role, confidence on findings, ref syntax, caps. Ref existence is left to `Board::append`. |
| `OllamaAgent` | `AgentRunner` for `Agent::Local` only. Runs `ollama run <model> --format json` with the prompt on stdin through a `CommandRunner`. |
| `CommandRunner` | The process seam: fixed argv, stdin bytes, deadline. `StdCommand` is the real one; tests use a fake. |

The protocol and the reasons behind it are in [ADR-0004](../../docs/adr/0004-ollama-output-protocol.md).

## Run it
```sh
ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-ollama --example research -- "How does Plan::start prevent self-review?"
```

## Test it
```sh
cargo test -p orch-ollama                 # pure functions, fake runner, and StdCommand over cat/sh/sleep
ORCH_OLLAMA_MODEL=llama3.2 cargo test -p orch-ollama --test real_ollama -- --ignored
```
The ignored test runs the real `ollama` binary and passes only if the model read the prompt from stdin under `--format json` and returned at least one well-formed entry.

## Dependencies
`orch-core`, `orch-dispatch`, and `rusty_json` with default features off (no serde, no registry crates).
