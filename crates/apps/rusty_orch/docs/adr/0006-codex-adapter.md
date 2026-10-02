# ADR-0006: Codex adapter: `codex exec`, read-only sandbox, schema-constrained reply

- **Status:** Accepted
- **Date:** 2026-10-02

## Context
`Agent::Codex` is the second CLI adapter. Codex authenticates with the ChatGPT subscription login (ADR-0001: never a vendor API key). Unlike Ollama, Codex can read the repository itself, so refs can stay refs. Every claim below was verified against codex-cli 0.160.0 installed from npm, not recalled from memory.

## Decision
### Invocation
Fixed argv, no shell, no interpolation, prompt on stdin:

```
codex exec --sandbox read-only --ephemeral --ignore-user-config --ignore-rules -C <repo_root> [-m <model>] --output-schema <schema> --output-last-message <reply> -
```

- `-` reads the prompt from stdin (`exec --help`: "If not provided as an argument (or if `-` is used), instructions are read from stdin").
- `--sandbox read-only`: writes under the working directory fail with "Read-only file system", outbound network fails to connect, reads of relative and absolute paths succeed (all exercised through `codex sandbox`). `codex exec` has no approval flag; its header prints `approval: never`, so nothing can prompt.
- `--ephemeral` keeps session files off disk. `--ignore-user-config` stops a developer's `~/.codex/config.toml` from changing the adapter's behaviour; auth still works.
- `--ignore-rules` is a separate flag and is as load-bearing as the sandbox. Without it Codex still loads `$CODEX_HOME/rules/*.rules` and project `.rules` files; a saved `prefix_rule(..., decision="allow")` that matches a command sets `bypass_sandbox`, and the first attempt then runs outside the read-only sandbox. Verified offline with `codex execpolicy check`: a `sh -c` allow rule decides `allow` for a file write. The adapter therefore never loads user or project rules. Found in review; the first cut had only `--ignore-user-config`.
- `-C <repo_root>` is the configured repository root. Codex reads files relative to it. `--skip-git-repo-check` is deliberately omitted: a misconfigured root fails loudly instead of running Codex against the wrong directory.
- No `--skip-git-repo-check`, no `--json`, no `--dangerously-*` flags.

### Reply
`--output-schema` constrains the model's final response to [`OUTPUT_SCHEMA`](../../crates/orch-codex/src/schema.rs), which mirrors what `orch_cli::parse` accepts. Codex forwards the schema as a **strict** Structured Outputs schema, so every object must list every property in `required` and set `additionalProperties: false`, and optional fields do not exist. Kind-dependent fields (`confidence` on findings, `verdict` on reviews) are one fully-required `anyOf` variant per kind. Size caps and string lengths are left out of the schema and enforced by the parser, so the schema never depends on which extra keywords a provider accepts. A contract test walks the schema and fails on any object that breaks the strict rules. Found in review; the first cut declared `confidence` and `verdict` as optional properties, which strict mode rejects before any reply is produced. `--output-last-message` writes that response to a scratch file. The adapter reads the file and ignores stdout, which otherwise carries a transcript or JSONL events. Both scratch files live in the OS temp dir, one pair per run, removed on drop. The parser remains the single source of truth: it decides which kinds a role may write and whether confidence or verdict is required.

### Environment
`OPENAI_API_KEY` is removed from the child's environment on every run (`CommandRunner::run_scrubbed`). In a container with that variable set, Codex silently took the paid API path; the scrub makes the subscription login the only path.

### Failure mapping
Codex exits 1 for both a missing login and an exhausted quota, so stderr tells them apart. A missing login ends with `401 Unauthorized`; quota exhaustion carries `429`, `rate_limit_reached` or `usage_limit_reached`. The adapter maps these to `AgentError`s that begin `codex: not logged in` and `codex: rate limited`, each with a bounded single-line stderr excerpt. A sandbox refusal is not a process failure: Codex returns the denied command to the model, which reports it in a normal reply. Timeout and overflow come from the shared seam with the group kill and bounded join (ADR-0004).

### Protocol addition: the review kind
The example routes a review of Codex's finding to a local model, so the shared protocol gains `review` for `Role::Review` cards: `{"kind":"review","verdict":"approve"|"changes_requested","body":...,"refs":[...]}`, parsed to `EntryKind::Review { of: <the card's target>, verdict }`. A review card must contain exactly one review entry and may add findings beside it. This is an amendment to ADR-0004 and applies to every adapter.

## Consequences
- Read-only is the only mode. Write access, the Implement role, worktrees and MCP access to the board are out of scope by choice and would each need their own ADR.
- A real rate limit was never triggered during development; the mapping rests on the binary's own strings. If Codex changes that wording, the error degrades to a generic exit-1 message, never a wrong classification. A non-retryable error variant in `orch-dispatch` is the follow-up that would make this stricter.
- The ignored tests and the example are the only places the real binary runs, and they run on the developer's machine; CI covers everything through fakes. Two of the ignored tests are about rules: one needs only the binary and shows the allow-rule hazard through `codex execpolicy check`; the other needs a login and proves the adapter's argv keeps a saved allow rule from writing a file.
