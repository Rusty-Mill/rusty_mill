# Release Notes

One entry per merged PR against `main`, newest first. No version tags yet.

---

## Superseded refs resolve to their live successors (#439)
**2026-10-02** · (link once pushed)

- **Fixed:** the shared prompt renderer follows an explicit entry ref through the board's linear supersession chain and renders the newest live successor's actual id, kind, and body instead of silently dropping the context.
- **Fixed:** duplicate refs and refs to several ancestors of one chain render that live entry once. Deduplication tracks entries actually rendered, so the same entry is not repeated under "THIS CARD SO FAR," while resumed-card questions and their linked human answers remain visible.
- **Added:** shared-renderer regressions for zero-hop, multi-hop, alias, isolation, and history deduplication, plus Ollama and Codex fake-runner coverage of the prompt each adapter receives.

---

## Resumed cards see their answers (#448)
**2026-10-02** · (link once pushed)

- **Fixed:** `orch_cli::render` only inlined entries the card's immutable refs pointed at, so a card resumed after a `Question` was re-run with no sign its question had been answered. The prompt now ends its context with the card's own live entries and any live `Answer` to them, minus what the refs already show, under a "THIS CARD SO FAR" heading that appears only when there is something to show. Every adapter inherits it. Found by the repository audit; the dispatcher test proved the state transition and the second call but never looked at the prompt.
- **Added:** render test with a Question tagged to the card and an untagged human Answer, plus an `orch-ollama` adapter test asserting the resumed prompt on the fake's stdin carries the answer.

---

## Second real agent — orch-codex
**2026-10-02** · (link once pushed)

- **Added:** `orch-codex`, `Agent::Codex` over the Codex CLI, read-only. Fixed argv `codex exec --sandbox read-only --ephemeral --ignore-user-config --ignore-rules -C <repo_root> [-m <model>] --output-schema <f> --output-last-message <f> -`, prompt on stdin; the reply is read from the last-message file, constrained by `OUTPUT_SCHEMA` in strict Structured Outputs form (one fully-required `anyOf` variant per kind; caps left to the parser), and parsed by the shared parser. `--ignore-rules` keeps a saved execpolicy allow rule from running a command outside the sandbox. Default timeout 10 minutes. Every flag verified against codex-cli 0.160.0.
- **Added:** `OPENAI_API_KEY` is scrubbed from Codex's environment on every run, so the subscription login is the only path. Failures map to typed `AgentError`s: `codex: not logged in` (stderr `401 Unauthorized`), `codex: rate limited` (`429`, `rate_limit_reached`, `usage_limit_reached`), generic exit status otherwise, plus the shared timeout and overflow. A sandbox refusal is a normal reply, not an error.
- **Added:** the `review` kind in the shared protocol: a `Role::Review` card writes exactly one `{"kind":"review","verdict":...}` entry and may add findings. Lets a local model review Codex's finding in the `research_review` example, the first run where two real adapters share one board.
- **Added:** tests: argv pinned; schema is strict-compatible (recursive check: every object fully required, no additional properties, no keywords outside the strict subset) and agrees with the parser on both ADR-0004 example replies; two ignored rules tests (offline `execpolicy check` shows the allow-rule hazard; end-to-end isolated `CODEX_HOME` with a saved allow rule must not write a file); render keeps `path:` refs as refs; fake-runner cases for success (with scrub and scratch-file cleanup), wrong agent, not logged in, three rate-limit shapes, sandbox refusal, other exit status, timeout, malformed reply, missing last-message file; parser tests for the review kind. One `#[ignore]` test runs the real binary with a nonce that must appear in an entry body.
- **Added:** ADR-0006 recording the invocation, sandbox, schema path, env scrub, failure mapping, and the review-kind amendment to ADR-0004.
- Out of scope, by choice: write access or the Implement role for Codex, worktrees, Claude and Gemini adapters, MCP access to the board, a non-retryable error variant.

---

## Shared CLI-adapter core — orch-cli
**2026-10-02** · (link once pushed)

- **Added:** `orch-cli`, extracted from `orch-ollama` now that Codex is the second CLI adapter. It holds the process seam (`CommandRunner`, `StdCommand`, `Exit`, `ExecError`, `JOIN_GRACE`, `MAX_STDOUT_BYTES`), the JSON reply protocol (`parse`, `allowed_kinds`, caps), the prompt core (`render(task, board, footer)`, `format_spec(role)`), and the `fake` module (`FakeCommand`, board fixtures, the ADR-0004 example replies). Files moved with `git mv`; no logic changed.
- **Added:** `CommandRunner::run_scrubbed(argv, stdin, timeout, remove_env)`. `run` is now the provided method calling it with an empty list. `StdCommand` removes the named variables from the child's environment. Motivated by Codex picking up a stray `OPENAI_API_KEY`; Ollama passes an empty list. One new seam test.
- **Changed:** `orch-ollama` is a thin adapter: argv, footer, and error mapping. Every existing test passes with only import paths changed; the one render-core test now runs in `orch-cli`, and the Ollama footer assertion stays in `orch-ollama`.
- **Changed:** ADR-0004 retitled to cover CLI adapters generally and amended to point at the new home.

---

## No self-approval, and the deadline is a hard bound
**2026-10-01** · (link once pushed)

- **Changed:** `orch-core` `Board::append` now requires that the live approving `Review` an agent cites for a `Decision` be written by a different author. A human's review counts for any agent; an agent's own never counts for itself; one qualifying review among several self-approvals is enough. Same `BoardError::DecisionNeedsApproval`. Five new tests; ADR-0005 carries the author rule.
- **Changed:** `orch-ollama` `StdCommand` spawns the child as leader of its own process group and, on timeout or overflow, kills the whole group through the system `kill` (unix) or `taskkill /T` (Windows) with a fixed argv, then reaps the leader. Pipe reader threads are joined for at most `JOIN_GRACE` (2s) and detached otherwise; `Timeout` and `StdoutOverflow` take precedence over any pipe-join error. A child that exits while a grandchild holds stdout fails within the grace period instead of blocking. No `libc` or `windows-sys` dependency, so the crate stays registry-free. Windows path compiles but is not exercised in CI. Three new unix tests: a forking child with a 1s timeout returns `Timeout` in under 3s and leaves no group member behind; an overflow with a forking child returns within the bound; a grandchild holding stdout after exit is bounded by the grace period. ADR-0004 updated.

---

## Agents never settle decisions alone
**2026-10-01** · (link once pushed)

- **Added:** a `Board` invariant in `orch-core`. `Board::append` accepts `EntryKind::Decision` from `Author::Human` unconditionally, and from `Author::Agent` only when `refs` includes a live (not superseded) `Review` with `Verdict::Approve`; otherwise `BoardError::DecisionNeedsApproval`. Supersessions follow the same rule, so an agent cannot overwrite a human's decision without an approval behind it. Enforced in the board so every adapter inherits it.
- **Changed:** `orch-ollama` no longer offers `decision` to any role (Design had it). Agents propose decisions as findings; the prompt's format spec says so.
- **Added:** ADR-0005 recording the policy. Seven new `orch-core` tests: human decision without refs, agent decision without refs, backed by an approving review, backed by changes-requested, backed by a superseded approval, backed by a non-review entry, and agent supersession of a human decision with and without approval. Existing dispatcher tests unchanged.
- Out of scope, by choice: a `Proposal` kind, routing changes, auto-promotion of approved findings.

---

## First real agent — orch-ollama
**2026-10-01** · (link once pushed)

- **Added:** `orch-ollama`, `Agent::Local` over `ollama run <model> --format json` with the prompt on stdin. Three pure functions plus a thin shell: `render(task, board)` builds the prompt from the card, the live entries its refs point at, and the format spec for its role; `parse(stdout, role)` is strict (allowed kinds per role, confidence required on findings, ref syntax checked, 8 entries and 500-character bodies as caps, one fence tolerated); `OllamaAgent` maps non-zero exit, timeout, overflow, spawn failure, and empty stdout to `AgentError` with a bounded stderr excerpt.
- **Added:** `CommandRunner`, the adapter's own process seam, because `contract::ProcessRunner` has neither stdin nor a timeout. `StdCommand` writes stdin on its own thread, drains both pipes concurrently, polls against a deadline, kills and reaps on timeout or when stdout exceeds 1 MiB.
- **Added:** `cargo run -p orch-ollama --example research -- "<question>"` builds a one-card Research plan, runs the dispatcher with `OllamaAgent`, and prints the live board. An ignored integration test (`ORCH_OLLAMA_MODEL=... cargo test -p orch-ollama --test real_ollama -- --ignored`) confirms the real binary reads the prompt from stdin under `--format json`.
- **Added:** ADR-0004 recording the JSON protocol, the caps, the fence tolerance, the `rusty_json` choice under root ADR-0002, and the local process seam.
- Dependencies: `orch-core`, `orch-dispatch`, `rusty_json` (default features off; no registry crates). Tests cover render inclusion and exclusion, both ADR example replies, every malformed case, caps, the fake runner's five failure paths, and `StdCommand` over `cat`, `sh`, `sleep`, and `head`.
- Deliberately deferred: Codex, Claude and Gemini adapters, a generic CLI-adapter abstraction, MCP access for agents, Implement and Review roles on Local.

---

## In-memory dispatcher — orch-dispatch
**2026-10-01** · (link once pushed)

- **Added:** `orch-dispatch`, the application layer over `orch-core`. `AgentRunner` is the port: one call per invocation, returns `Output` entries that the dispatcher stamps with task and author before appending, so an adapter can neither skip board validation nor write as another agent.
- **Added:** `Routing`, validated from `RoutingConfig` at construction. Work roles map to one agent each; reviews take the first agent in an ordered preference list that is not the target's author, and the list must hold at least two distinct agents so every author can be reviewed.
- **Added:** `Dispatcher::run(goal, plan, board, ledger)`: deterministic loop over `Plan::ready`, one card at a time in `TaskId` order (a deliberate P1 choice; parallel fan-out is the trigger for async later). Each step is check → start → count → run → record. A `Question` entry blocks the card and the run returns `Outcome::Blocked`; after a human `Answer` the next run resumes it, and `Done.outputs` then holds only the final call's entries. Zero outputs surface `PlanError::NoOutputs`.
- **Added:** `Ledger`, caller-owned call accounting for `Budget::max_calls` and each card's `TaskSpec::max_calls`. The dispatcher holds only routing and the runner, so a fresh dispatcher resuming with the same ledger still honours what earlier runs spent. Exceeding either ceiling returns `DispatchError::CeilingReached` naming the ceiling and the card with nothing spent.
- **Added:** metered retry with exhaustion. An agent failure leaves the card `Running` and counted, so the next run retries it. When a `Running` card is refused by its task ceiling the dispatcher calls `Plan::fail` with the refusal as the reason; a later run with that card's dependents stranded returns `DispatchError::Stuck`.
- **Added:** atomic board writes per call: outputs are staged on a copy of the board and swapped in only if every append validates, so a rejected output leaves the board unchanged and the card `Running`.
- **Added:** `FakeAgent` scripted fixture behind the `fake` feature (rk-kernel convention); 11 integration tests cover the full research → implement → review pipeline, reviewer fallback, question block/resume, both ceilings, zero outputs, agent failure and retry, retry exhaustion stranding a dependent, an invalid output leaving the board unchanged, and a fresh dispatcher honouring an existing ledger.
- **Added:** ADR-0003, accepted: the dispatcher depends only on `orch-core` because the workspace layer checker forbids apps-to-apps cross-family edges; `contract::ProcessRunner` and `rusty_sqlite` are the allowed future edges.
- **Fixed:** `SECURITY.md` advisory link now points at `Rusty-Mill/rusty_mill`.
- Deliberately deferred: real CLI adapters, persistence, async, wall-clock metering, retry policy, board/plan task-id cross-checks.

---

## Initial import — orch-core domain crate and repo governance
**2026-10-01** · (link once pushed)

- **Added:** `orch-core`, a zero-dependency domain crate with three aggregates: validated goal contracts (`Goal::try_from` rejects drafts missing acceptance criteria, out-of-scope, or budget, reporting all problems at once); task cards with `Plan` as the sole mutation point (acyclic by construction, implicit review prerequisites, no self-review, completion requires ≥1 board entry); and an append-only `Board` with same-kind supersession and referential integrity on append.
- **Added:** governance set (README, ARCHITECTURE, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT, PR/issue templates, Rust CI, `.gitattributes`), `AGENTS.md` as shared agent context with `CLAUDE.md` importing it, ADR-0001 (shared substrate over agent messaging) and ADR-0002 (single domain crate).
- Deliberately deferred: retrying failed tasks, typed goal refs (goal refs are still plain text), board/plan task-id cross-checks, timestamps and budget metering (adapter concerns). No adapters or dispatcher yet.
- 27 unit tests; all pass. fmt and clippy `-D warnings` clean.
