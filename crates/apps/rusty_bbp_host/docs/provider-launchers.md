# Codex author and Claude reviewer

Build with `cargo build --locked -p rusty_bbp_host`. This produces `bbp`,
`codex-bbp-harness` and `claude-bbp-harness`. These launchers require Linux.
Install/authenticate Codex and Claude Code with their subscription logins; the
launchers do not log in, copy credentials, or support an API-key fallback.

The moderator launches them with its existing per-turn environment. In an
Orch pool use `program: "codex-bbp-harness"` or `"claude-bbp-harness"` and
`args: ["--mcp-config", "{mcp_config}"]`. Add `"--model", "MODEL"` to pin a
model or `"--provider-bin", "/absolute/path/to/cli"` to choose the installed
executable. The latter is a trusted executable selection, not a sandbox for
arbitrary wrappers. Other flags and duplicate options are refused. The
[Orch example](../../rusty_orch/docs/examples/bbp.json) is a two-role roster.
Codex needs a non-final role capable of Spec/Diff/Candidate production and test
verdicts; Claude needs an independent scoped final reviewer with no writes or
raw-log access. Role ids are configuration data, not hardcoded names. Set the
author assignment's `builtin_tools` to true and the reviewer's to false.

Both launchers refuse stale turns, incorrect principals/vendors and foreign or
modified MCP configurations before provider startup. Private files live under
`<BBP_DIR>/harness/<task hash>/turn-N/`; a turn workspace is never reused.
Each invocation starts a fresh provider session and uses only the bound BBP
MCP server. On Linux the launcher replaces itself with the provider process,
preserving the moderator's process-group supervision and the provider's exit
status. Provider startup errors remain errors; there is no scripted fallback.

Codex receives a fresh local clone with no push remote and a workspace-write
sandbox. User config and exec rules are ignored; the clone is untrusted for
project config, hooks/plugins/delegation are disabled, and a required inline
TOML MCP server carries the exact task/principal/turn environment. No global
`codex mcp add` or user-config edit occurs. The author restores prior candidate
diffs when revising and submits artifacts through MCP. The authoritative BBP
runner still applies and validates the candidate independently under confinement.

Claude receives an empty working directory, `--tools ""`, `--restricted`,
`--strict-mcp-config`, empty setting sources, disabled hooks/skills, no session
persistence and `dontAsk` permissions with no interactive permission prompts.
Only task_card/read/get_artifact/post BBP calls are preapproved. The BBP engine
also enforces the scoped, read-only roster and prevents raw-log access. The
launcher, not merely `builtin_tools: false` metadata or the prompt, supplies
the tool restrictions. It refuses a same-vendor candidate writer.

Provider CLIs and administrator-managed policy remain trusted. The author CLI
has local shell/file tools in its own clone; the reviewer has none. This does
not claim an OS sandbox around a malicious provider executable. Host state and
credentials must stay outside the runner workload's granted read roots. API-key
and push-token variables and inherited BBP bindings are removed from provider
environments; the MCP config explicitly supplies only its four public binding
variables. Provider auth stays in the CLI's normal login store.

## Compatibility and tests

The fixed arguments were checked against installed Codex CLI 0.155.1 and Claude
Code 2.1.302 help. Unsupported flags fail startup; restrictions are never
silently dropped for older CLIs. Contract tests use provider stubs and make no
model calls. Run them with:

```sh
cargo test --locked -p rusty_bbp_host --lib launchers
cargo test --locked -p rusty_bbp_host --test provider_launchers
```

Sources for the command contracts:
[Codex CLI](https://learn.chatgpt.com/docs/developer-commands?surface=cli),
[Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference),
[Claude CLI](https://code.claude.com/docs/en/cli-reference).

## Opt-in real-provider Linux smoke

This test makes subscription model calls; it is ignored in ordinary CI. Choose
models available to the two installed/logged-in providers. No real merge or push
is performed. The fixture creates a tiny disposable repository and uses the
real Orch CLI, persisted state, production supervisor, confined runner and both
provider launchers through plan approval, candidate validation, independent
review, merge approval and a clearly marked test-only receipt. It fails on a
refused sandbox or missing provider progress. Model behavior is nondeterministic;
retain the temporary task's agent/moderator logs when investigating a failure.

```sh
cargo build --locked -p rusty_bbp_host -p rusty_orch
BBP_LIVE_SMOKE=1 \
BBP_CODEX_MODEL='YOUR_CODEX_MODEL' \
BBP_CLAUDE_MODEL='YOUR_CLAUDE_MODEL' \
BBP_TEST_BIN="$PWD/target/debug/bbp" \
ORCH_TEST_BIN="$PWD/target/debug/rusty_orch" \
cargo test --locked -p orch-bbp --lib \
  tests_confined::opt_in_codex_author_claude_reviewer_smoke \
  -- --ignored --exact --nocapture
```

The four deterministic confined acceptance cases remain required CI. Successful
offline contracts do not establish successful live-provider behavior; report
the smoke's actual result separately.
