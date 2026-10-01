# Release Notes

One entry per merged PR against `main`, newest first. No version tags yet.

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
