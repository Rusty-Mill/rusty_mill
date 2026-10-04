# orch-codex

`Agent::Codex` for rusty_orch, over the Codex CLI, read-only. A thin adapter on `orch-cli`; the protocol and the process seam live there. See [ADR-0006](../../docs/adr/0006-codex-adapter.md).

| Piece | What |
| ---- | ---- |
| `CodexAgent { repo_root, timeout, model }` | `AgentRunner` for `Agent::Codex` only. Runs `codex exec --sandbox read-only --ephemeral --ignore-user-config --ignore-rules -C <repo_root> --output-schema <f> --output-last-message <f> -` with the prompt on stdin, reads the reply from the last-message file, parses it with `orch_cli::parse`. Default timeout 10 minutes. |
| `output_schema(stop)` | The JSON Schema handed to `--output-schema`, re-exported from `orch-cli` where the Claude adapter shares it: strict Structured Outputs form, one fully-required `anyOf` variant per kind, no `question` variant under best effort. The parser remains the authority on caps and roles. |
| `render(task, board, stop)` | The shared prompt core plus two Codex lines: read files under the working directory and cite them as `path:` refs; the sandbox is read-only and offline. |
| `SCRUBBED_ENV` | `OPENAI_API_KEY` is removed from the child's environment on every run. Codex uses the ChatGPT subscription login, never a key. |

Failures are typed `AgentError`s whose messages tell "not logged in" from "rate limited"; both are exit 1 from Codex and are told apart by stderr. "Not logged in" is classified as an unavailable prerequisite: the card remains resumable and the attempt does not spend call budget. Rate limits are counted transient failures.

## Run it
Needs `codex` on `PATH` and `codex login status` reporting a login, plus `ollama` for the reviewer.
```powershell
$env:ORCH_OLLAMA_MODEL = "llama3.2"
cargo run -p orch-codex --example research_review -- "How does Plan::start prevent self-review?"
```
Codex researches the question against the repository; a local model reviews the finding; the live board is printed.

## Test it
```powershell
cargo test -p orch-codex                      # pure facts and the fake runner
$env:ORCH_CODEX_REPO = (Get-Location).Path
cargo test -p orch-codex --test real_codex -- --ignored --nocapture
```
The ignored test puts a nonce in the instruction and passes only if a returned entry body carries it, which proves stdin delivery, the schema plus last-message path, `-C`, and the subscription login in one run.

```powershell
cargo test -p orch-codex --test rules -- --ignored --nocapture
```
Two rules tests. The first needs only the binary: it shows Codex's own `execpolicy check` deciding `allow` for a file write under a saved `sh -c` allow rule, the hazard `--ignore-rules` exists for. The second needs a login: an isolated `CODEX_HOME` (auth copied in, the allow rule saved as `rules/default.rules`), a model told to run exactly that write, and the adapter's argv; the file must not appear.
