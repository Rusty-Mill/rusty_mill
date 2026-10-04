# Release Notes

One entry per merged PR against `main`, newest first. No version tags yet.

---

## Resume a blocked run: `--state <dir>`
**2026-10-03** · (link once pushed)

- **Added:** `orch-store`, persistence over the embedded `rusty_multimodal_db_engine` (libs layer, the allowed edge). One snapshot record per goal holds the plan, the board, the ledger, and a fingerprint of the goal file; a save is one durable `replace`, so no journal is needed. `load` rebuilds the state through `Plan::add` and the lifecycle transitions, `Board::append`, and the new `Ledger::from_counts`, so every `orch-core` invariant is checked again and a bad snapshot is refused as corrupt. The directory is locked per process.
- **Added:** `rusty_orch run <goal.json> --state <dir>` (env `RUSTY_ORCH_STATE`). The state is saved after every dispatcher run and every answer round; the next run on the same directory and goal file continues from it, and calls present in the recovered checkpoint count against both ceilings. Abrupt termination can repeat work and calls since the checkpoint. A goal file that no longer matches the saved fingerprint is refused (exit 1). Without the flag the run stays in memory as before. The wall clock is per invocation, so a goal waiting on a human does not spend it.
- **Changed:** `orch-dispatch`: additive `Ledger::from_counts`. ARCHITECTURE's board-store row and ADR-0003's `rusty_sqlite` row are superseded. Root manifest gains the `orch-store` member and entry; `docs/WORKSPACE-MAP.md` regenerated.
- **Deviation, stated:** `orch-store` depends on `serde` (the engine's record bound) and carries the engine's pinned registry crates transitively; it and the dependent binary require Rust 1.89. The initial positional-bincode format is `GoalRecord::v1`; layout changes require an explicit reader/migration or a new rejecting tag.
- **Tests:** store round trip through reopen; deterministic insert- and replace-after-log recovery; populated v1 golden bytes, corrupt-snapshot reconstruction, and wrong-tag rejection; partial-answer I/O failure persistence; repeated in-process entry-point resume calls; and a real two-process binary reopen using a scripted local CLI. The injected faults are not OS process-crash tests. ADR-0010.
- Out of scope, by choice: per-step saving (needs a dispatcher seam), multi-goal directories, a separate answer command, Claude and Gemini adapters, `StopRule::BestEffort`.

---

## The command: `rusty_orch run`
**2026-10-03** · (link once pushed)

- **Added:** the `rusty_orch` binary. A JSON goal file carries the goal contract by `GoalDraft` field name, a human-authored `tasks` array (`depends_on` and a review's `target` as indices into earlier tasks, refs in the adapter protocol's `path:`/`commit:`/`url:`/`E-n` syntax), and an optional `routing` object defaulting to Codex for research, design, and implement, the local model for triage, reviewers `local` then `codex`. Every rejection names the JSON path; goal-contract problems are reported together.
- **Added:** the run loop checks the goal's wall clock before each dispatcher run, prints one progress line per run to stderr (a fixed outcome category plus task id, agent, and call count; adapter and model error text stays in the report), and on a block either stops with exit status 3 or, with `--interactive`, offers each open question on stderr, appends the stdin answer as a Human `Answer`, and runs again. A blank line or end of input stops the round at once, keeping earlier answers and making no further model call. stdout carries only the report, so `--json` stays one parseable object. Dispatcher errors end the run with the state intact for the report (exit 4).
- **Changed (`orch-cli`):** `render` gives a `Role::Review` card the reviewed task's live entries and the answers to them under `UNDER REVIEW`, resolved at render time, so a review verdict is always given on the actual output (including findings written after a question/answer/resume) without the author naming entry ids.
- **Added:** text report (goal, ended, calls, cards, live board) and `--json` (one object with `goal`, `ended`, `calls`, `tasks`, every entry with a `superseded` flag). The composite runner forwards `run_classified` so Codex's recoverable login reaches the dispatcher.
- **Changed:** `orch_cli::parse_ref` is public; `orch-codex` gets a `[workspace.dependencies]` entry; `docs/WORKSPACE-MAP.md` regenerated.
- **Tests:** progress lines exclude adapter and model text (sentinel strings in a synthetic agent error, a child's stderr, and a malformed reply reach the report but never stderr progress, with exit status 4 intact); argument parser (defaults, env, flags, every usage error); goal file (example parses and builds a plan, defaults, grouped goal problems, twelve path-named rejections, forward and self references refused at plan build, non-object `routing` rejected for string/null/array/number/boolean while omitted and partial objects keep defaults); run loop over `FakeAgent` (finished with both reports, blocked without answers, answer resumes, blank answer ignored, a stop after an earlier answer makes no further call and keeps the answer, wall clock before the second run, dispatcher failure keeps state, goal ceiling); entry point over captured streams (`--interactive --json` through a blocked-and-answered run yields one parseable JSON object on stdout with the question on stderr; blank stdin exits 3; non-interactive never reads stdin); renderer (review card sees multi-entry target output, sees the target's question, answer, and resumed finding once, says `(none)` for a silent target). No real binary run. ADR-0009.
- Out of scope, by choice: persistence and cross-process resume, `StopRule::BestEffort` behaviour, Claude and Gemini adapters, the Implement role.

---

## Classified agent failures preserve login recovery
**2026-10-03** · (link once pushed)

- **Fixed:** Codex `401` / `not logged in` is an unavailable external prerequisite, not a permanent card failure. Repeated explicit runs leave the same card `Running` and do not consume call budget; after login, that card can succeed normally, including with `max_calls = 1`. No internal retry, partial output, or fabricated success is introduced.
- **Changed:** the additive `ClassifiedError` policy returned by `AgentRunner::run_classified` distinguishes transient, permanent, and unavailable failures. Counted transient errors retain ceiling behavior and true permanent failures still fail immediately. The dispatcher never classifies message text.
- **Compatibility:** `AgentError(String)`, its `.0` field, existing `AgentRunner::run` implementations, `DispatchError::Agent`, and persisted core state are preserved. Existing runners default to counted transient behavior. `FakeAgent::Reply` keeps its two variants; dispatcher tests script permanent and unavailable outcomes with a test-local typed runner, and pin that a composite forwarding only `run` erases the inner classification. ADR-0008 amends ADR-0007.

---

## Unsupported adapter roles fail once (#440)
**2026-10-02** · (link once pushed)

- **Fixed:** `AgentRunner::supports` gives adapters an explicit capability check. The dispatcher uses it before counting a call or invoking the backend; an unsupported agent/role pair becomes a typed `UnsupportedRole` error and a terminally failed card, so later runs do not retry it.
- **Preserved:** ordinary `AgentError`s remain transient and retry under both goal and task ceilings. Ollama and Codex still serve Research, Design, Triage, and Review; both explicitly reject Implement before spawning a CLI, and Codex does so before creating scratch files.
- **Compatibility:** the new trait method defaults to supporting all routes, preserving existing in-repo and downstream runners. Capability-aware composite runners delegate the query to the selected adapter. No `orch-core` API or persisted shape changes.

---

## Ollama real-binary test proves stdin delivery (#441)
**2026-10-02** · (link once pushed)

- **Fixed:** `orch-ollama`'s ignored `real_ollama` test only checked that the reply was non-empty, which a model ignoring stdin could satisfy. It now builds a Research card whose instruction carries a per-run nonce and asserts some entry body contains it, mirroring `orch-codex`'s `real_codex` test. Test only; no library change.

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
