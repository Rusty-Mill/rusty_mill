# ADR-0012: Claude adapter: `claude -p`, read-only tools, schema-constrained envelope

- **Status:** Accepted
- **Date:** 2026-10-04

## Context
`Agent::Claude` has been in `orch-core` since ADR-0001 and goal files have accepted `"claude"` in `routing` since ADR-0009, but nothing served it: the composite runner answered "no adapter". The Codex adapter (ADR-0006) is the template: fixed argv, no shell, prompt on stdin, the CLI reads the repository itself, the reply is constrained to a schema, the vendor key is scrubbed, failures are classified. Claude Code authenticates with the Claude subscription login (ADR-0001: never a vendor API key). Every claim below was verified against Claude Code 2.1.289 (`claude --help`, `claude auth status`, and one live `claude -p` run against this repository); the Gemini adapter is a separate decision once its binary can be verified the same way.

## Decision
### Invocation
Fixed argv, no shell, no interpolation, prompt on stdin, the repository root as the working directory:

```
claude -p --output-format json --tools Read,Grep,Glob --permission-mode dontAsk --permission-prompts none --restricted --strict-mcp-config --setting-sources "" --disable-slash-commands --no-session-persistence --max-turns 20 [--model <m>] --json-schema <schema text>
```

- `-p` is print mode; with no positional prompt the prompt is read from stdin (capped at 10 MB by the CLI, far above any card). `--output-format json` prints one result envelope on stdout.
- Read-only is enforced by the CLI, not by an OS sandbox, through three independent layers: `--tools Read,Grep,Glob` leaves no edit, write, shell, or web tool in the session; `--restricted` removes the code-running tools and `WebFetch` even if a settings file names them, ignores user, project and local settings files, confines the file tools to the working directories, and refuses `bypassPermissions`; `--permission-mode dontAsk` with `--permission-prompts none` denies anything that would have prompted instead of waiting on a terminal nobody is watching.
- `--strict-mcp-config` with no `--mcp-config` loads no MCP server. `--setting-sources ""` and `--disable-slash-commands` keep a developer's settings, hooks, and skills out of the run. `--no-session-persistence` keeps session files off disk.
- The working directory is the configured repository root, passed through the new `cwd` parameter of `orch_cli::CommandRunner::run_in`, not inherited from wherever the orchestrator was launched; `--restricted` confines the file tools to it. Codex keeps its `-C` flag.
- `--bare` is deliberately not used: its documentation states that Anthropic auth is then strictly `ANTHROPIC_API_KEY` or an API-key helper and that OAuth is never read, which is the opposite of the subscription posture. `--safe-mode`, `--allowedTools`, `--add-dir`, and every `--dangerously-*` flag are also absent, and a contract test pins their absence.
- `--max-turns 20` bounds the agentic loop; `--max-budget-usd` applies only to API-key billing and is not used.

### Reply
`--json-schema` takes the schema as text in the argv and the CLI validates the final message against it. The schema is [`orch_cli::output_schema(stop)`](../../crates/orch-cli/src/schema.rs), **moved from `orch-codex` to `orch-cli`** in this change and re-exported by `orch-codex`, so the two schema-taking adapters share one text and `orch-claude` does not depend on `orch-codex`. Its strict Structured Outputs form (ADR-0006) is a superset of what Claude's validator needs. The result envelope carries `type`, `subtype`, `is_error`, `num_turns`, `result` (the final text) and, under `--json-schema`, `structured_output` (the validated object); the adapter re-serialises `structured_output` and hands it to `orch_cli::parse`, which remains the single authority on kinds, caps, and roles. Under `StopRule::BestEffort` the schema has no `question` variant (ADR-0011).

### Environment
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`, and `CLAUDE_CODE_USE_VERTEX` are removed from the child's environment on every run (`CommandRunner::run_in`). A key would route the run onto the paid API path; a base URL or provider toggle would send it to a proxy or a third-party provider. `CLAUDE_CODE_OAUTH_TOKEN` is not scrubbed: it is the subscription credential in headless setups.

### Login probe and failure mapping
Claude Code reports a missing login inside the result envelope, with `is_error: true` and text on stdout, not through a distinctive exit code. So each card run is two processes: `claude auth status` first, which exits non-zero when nobody is logged in and is mapped to the unavailable prerequisite (`claude: not logged in (run \`claude auth login\`)`, not metered, card left resumable, ADR-0008), then the run itself. The probe runs with the same scrubbed environment, so it cannot report a login the run would not have. The envelope is still read before the exit status: `is_error` with text naming a login (`not logged in`, `/login`, `authentication`, `401`) is unavailable; text naming a limit (`429`, `rate limit`, `usage limit`, `too many requests`, `overloaded`) is a counted transient failure named `claude: rate limited`; any other error, including `error_max_turns`, is a counted transient failure carrying the exit status and the envelope's `subtype`. A missing `structured_output` on a success envelope is a transient failure. Timeout and overflow come from the shared seam (ADR-0004).

### Command line
`--repo <dir>` (env `ORCH_REPO`) names the repository both Codex and Claude read; `--codex-repo` and `ORCH_CODEX_REPO` remain as aliases. `--claude-model <name>` pins a model as `--codex-model` does. Default routing is unchanged: Claude is reached by naming `"claude"` in a goal file's `routing`.

## Consequences
- `orch_cli::CommandRunner` gains the required method `run_in(cwd, argv, stdin, timeout, remove_env)`; `run_scrubbed` and `run` become provided methods over it. Every implementation is in this family (`StdCommand`, the fakes) and updated here. `FakeCommand` records the requested working directories.
- `orch_cli::output_schema` is the schema's home; `orch_codex::output_schema` is a re-export, so no caller changes. ADR-0006's reference to the Codex crate's schema file now points here.
- `rusty_orch::agents::AgentsConfig` renames `codex_repo` to `repo` and gains `claude_model`; `rusty_orch::args::Args` likewise. Source changes for external struct literals, stated in the release notes.
- The read-only promise rests on the CLI's own flags, which a future Claude Code release could rename or weaken; the contract test pins the argv and the ignored real-binary test exercises it. An OS-level sandbox for Claude would be its own decision.
- The live auth-failure text was not observed during development (the development container is always logged in); the probe's exit code is the load-bearing signal and the text match is a second line. If Claude Code changes its wording, the error degrades to a generic transient message, never a wrong classification.
- Out of scope, by choice: the Implement role, write access, MCP access to the board, `--add-dir`. A Gemini adapter is dropped, not deferred: the Gemini CLI is discontinued.
