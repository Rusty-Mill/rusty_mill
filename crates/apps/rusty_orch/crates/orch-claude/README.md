# orch-claude

`Agent::Claude` for rusty_orch, over the Claude Code CLI, read-only. A thin adapter on `orch-cli`; the protocol, the reply schema, and the process seam live there. See [ADR-0012](../../docs/adr/0012-claude-adapter.md).

| Piece | What |
| ---- | ---- |
| `ClaudeAgent { repo_root, timeout, model, max_turns, stop }` | `AgentRunner` for `Agent::Claude` only. Runs `claude auth status` (exit 0 or the card is unavailable), then `claude -p --output-format json --tools Read,Grep,Glob --permission-mode dontAsk --permission-prompts none --restricted --strict-mcp-config --setting-sources "" --disable-slash-commands --no-session-persistence --max-turns 20 [--model m] --json-schema <schema>` with the prompt on stdin and `repo_root` as the working directory, reads `structured_output` from the result envelope on stdout, parses it with `orch_cli::parse`. Default timeout 10 minutes. |
| `orch_cli::output_schema(stop)` | The reply schema handed to `--json-schema`, shared with Codex; no `question` variant under best effort. The parser remains the authority on caps and roles. |
| `render(task, board, stop)` | The shared prompt core plus two Claude lines: read files under the working directory and cite them as `path:` refs; the tools are read-only and there is no shell or network. |
| `SCRUBBED_ENV` | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX` are removed from the child's environment on every run, probe included. Claude Code uses the Claude subscription login, never a key or a third-party provider. |

Read-only is enforced by the CLI, not by an OS sandbox: no edit, write, shell, or web tool exists in the session (`--tools`), `--restricted` removes the code-running tools whatever a settings file says and confines the file tools to the working directory, and `--permission-prompts none` denies anything that would have asked. `--bare` is not used: it never reads the subscription login.

Failures are typed `AgentError`s. A failed login probe, or an envelope whose error text names a login, is an unavailable prerequisite: the card remains resumable and the attempt does not spend call budget. Rate limits are counted transient failures named `claude: rate limited`. Everything else carries the exit status and the envelope's `subtype`.

## Test it
```sh
cargo test -p orch-claude                      # pure facts and the scripted runner
ORCH_REPO=$(pwd) cargo test -p orch-claude --test real_claude -- --ignored --nocapture
```
The ignored test puts a nonce in the instruction and passes only if a returned entry body carries it, which proves stdin delivery, the schema and envelope path, the working directory, and the subscription login in one run.
