# Release Notes

One entry per merged PR against `main`, newest first. No version tags yet.

---

## Half-limit erosion cleanups
**2026-11-06**

- `rusty_orch consolidate` now also opens a cleanup when erosion since the
  last cleanup reaches half of Nano's approved limits. It stays off until
  limits are recorded with `bbp erosion approve`.

## Scheduled erosion cleanups
**2026-11-06**

- New `rusty_orch consolidate`. After 5 accepted claims on a repo, it
  opens a claims task that must lower erosion without raising the other
  share (D44). It prints a plan unless `--execute` is given. When both
  shares are already zero, it records a skip instead.
- Only the claim count triggers a cleanup for now. The half-limit trigger
  Nano chose (D114) follows in its own PR.

## Track records read the full claim history
**2026-11-05**

- `bbp why --claim` now shows each line's author minimum, what each
  reopen woke, evidence refs, candidate changes and holdout report results
  (#599, D112). Lens track records read them, so the conservative
  stand-ins of the first build are gone: golden sets now count as ground
  truth, passed or failed, and fewer jobs are left pivotal and unscored.

## `lens record` calibration report
**2026-11-05**

- `rusty_orch lens record` prints each lens's track record on one task
  as one JSON line: points, blind reviews assigned, scored and pending,
  pivotal jobs, and pivotal jobs scored against ground truth (#599,
  D111). It is D93's calibration report, and it changes nothing.

## Lens track records
**2026-11-05**

- Each lens now has a track record (#599). `lens run` saves every blind
  review job and shows each lens `record: P points over N blind reviews
  (S scored, V pivotal)`. A review that decided the outcome itself is
  scored only against ground truth (a failing golden set), and is
  otherwise counted as pivotal and left unscored (D93, D92).
- The record never credits a lens for an outcome it may have moved:
  until BBP's history says more, golden truth isn't read from it, and a
  line after a human reopen, or a job with a post whose answer was lost,
  counts as pivotal. A crash or a failed post never drops a job's posts
  from its record or counts them twice; a crash between posting and
  saving leaves the job unscored, never scored on part of its posts.

## Alien lens hunts in `lens run --alien`
**2026-11-05**

- `lens run --alien` runs the #708 alien review lens after the ordinary
  passes: one hunt per eligible claim it has not hunted yet, with its
  probes run sandboxed, and the rows kept for `lens alien report` (D99).
  It is off by default and posts nothing to the board.
- Until #596's value score exists only irreversible claims are hunted,
  and only once every review lens has had its blind look. A claim it
  can't hunt is reported on stderr and tried again on the next run.

## Alien lens runs its probes in a sandbox
**2026-11-05**

- orch can now build and run the alien review lens's probe tests itself,
  with no network, on a scratch copy of the candidate and, when a probe
  fails there, of its base (#708, D96). Nothing calls it yet;
  `lens run --alien` will.
- A hang or a killed run is never counted as a finding, and a candidate
  can't affect the base run or write outside its scratch tree.

## Alien lens reads the candidate's diffs
**2026-11-05**

- The lens runner can now read a claim's current code candidate and its
  diffs, and tell whether they touch Rust, for the #708 alien review
  lens (D98). Nothing calls it yet; `lens run --alien` will.
- Reads are charged to the lens, not the claim. A lens role that may not
  read candidates makes the claim ineligible instead of failing the run.

## Cost breaker and taint for lenses
**2026-11-05**

- New pure pieces for card #598, not wired into `lens run` yet. The cost
  breaker spots a task whose spend keeps rising while its claims stop
  moving; by default it only reports, and in `enforce` mode it asks for a
  `bbp pause`. The lens config takes an optional `breaker` key.
- Taint marks what a lens posts with the most sensitive class it was
  shown, so a cleared local lens can't pass sensitive input on to a
  cloud lens. Each task's taint table is stored in `taints.mmap`.

## Alien lens hunt pipeline
**2026-11-04**

- orch-lens can run one alien-lens hunt over the model port: list
  suspects, rank them, ask for up to 3 probe tests, and classify each
  probe's runs (#708). Nothing called it until `lens run --alien`.
- A hunt that spent its first call is always recorded, so its calls are
  never spent twice.

## Lens cards show the task's accept line
**2026-11-04**

- BBP claim cards now carry each claim's accept line and every evidence
  item's strength and note (#594). `lens run` reads them from the card,
  so it can start: it no longer needs the task's claims settings or
  reads evidence notes from message pages.

## `rusty_orch lens run`
**2026-11-04**

- New command that runs the configured lenses on one BBP claims task
  until the board goes quiet (#594). It calls real models, so don't run it
  until a live run is approved. A restart asks every lens's first pass
  again until the runner's memory is saved (next change).
- Each claim's accept line comes from its BBP card, never from assumed
  defaults. It exits nonzero when the runner itself fails (unsaved meter,
  refused post).

## Lens models are told how to answer
**2026-11-04**

- Every lens question now explains the answer format and exactly what
  that question may post (#594). Before this, any model's answer would
  have been refused.

## The lens runner can talk to BBP
**2026-11-04**

- Each lens now has its own BBP session (#594), so it reads the board
  and posts under its own identity, with what it may see limited the
  same way as for any agent. Nothing starts these sessions yet.

## The lens runner can run a round
**2026-11-04**

- The lens runner's core can now run one full round (#594): pick the
  jobs the rules allow, ask each lens's providers, post the answers to
  the board and remember what happened. It is not wired to a command or
  to BBP yet.

## The lens meter is saved in the state directory
**2026-11-04**

- The lens money meter is now stored with the rest of rusty_orch's state
  (#594), one record per experiment, so a restart picks up its spend and
  charges any call that was in flight.
- Don't roll rusty_orch back past this release once a meter is saved.

## The lens meter survives a restart
**2026-11-03**

- The money meter can be saved and restored (#594). After a crash, any
  call that was still in flight is charged its full reserved worst case,
  so a restart never under-counts spend.
- Every lens call saves the meter after reserving and before asking the
  provider. If that save fails, the call is not made.

## Lens jobs fall back and are metered per call
**2026-11-03**

- One lens job now has a call step (#594). It tries the job's providers in
  order and fits the context to each one's window. It reserves the worst
  case before every ask and settles what the reply cost. An unavailable
  provider, a context that does not fit, or the experiment ceiling on a
  paid provider moves the job on. A bad reply or a task ceiling ends it.
- Replies are capped at 4096 output tokens on every provider. A longer
  reply is refused and still paid for.

## `rusty_orch lens check`
**2026-11-03**

- A dry run of the lens provider setup (#594): it loads the config and the
  operator's assertions file (default `/etc/let-em-loose/assertions.json`),
  builds every provider and prints one line each saying whether it would
  be admitted and what is still checked at call time. No model is called
  and no container is inspected.

## Codex lens questions run only in the checked container
**2026-11-03**

- Before every Codex lens question the runner inspects the operator's
  container on its own local podman and refuses anything but the asserted
  mounts and machine id, an unprivileged container with no extra devices,
  capabilities or shared namespaces, and no API key in its environment
  (#594). A failed check is `Unavailable` and runs nothing.
- The login probe and `codex exec` run with `podman exec` by the checked
  container id.

## Lens assertions can name a container
**2026-11-03**

- The operator's `host_cleared` entry may name a container and its exact
  mounts, including a read-only bind of `/etc/machine-id` (#594). Nothing
  runs under it yet; the per-call container check follows.
- Under a container clearance, a machine-bound no-overage entry must name
  the container's id, and Claude gets no grant.

## orch-codex: lens replies from the JSON events
**2026-11-03**

- A lens question now reads Codex's reply from the `--json` event stream
  it already reads for usage, so no reply file is written on the host and
  Codex can run inside the `lel-codex` container (#594).
- `CodexAgent::lens_argv()` no longer takes a reply path.

## orch-lens: the lens runner's core
**2026-11-03**

- New pure crate holding the lens runner's decisions: which jobs run, what
  each provider may see, what it may post back and what it may spend
  (ADR-0021). Nothing calls it yet; the `rusty_orch lens` shell follows.

## orch-openai: local model servers over HTTP
**2026-11-03**

- New adapter crate for OpenAI-compatible servers such as llama-swap:
  plain HTTP only, and only to loopback or private addresses, checked on
  every call against the address it connects to (ADR-0020).

## Providers are ids, not an enum
**2026-11-03**

- `orch_core::task::Agent` is now `AgentId`, a validated token, so a new
  model backend is configuration rather than a core change (ADR-0019).
- Snapshots write the id; the four old variants still load as `claude`,
  `codex`, `gemini` and `local`. An older binary cannot read a snapshot
  saved by this one.
- Errors and progress lines print the id (`codex failed T-1`).

## Leases bound validator roots; leased cancel
**2026-11-02**

- A lease now bounds the validator profile's read roots: each must be a
  system root or lie within the lease, and not beneath a harness's write
  root, and it is frozen as the path it resolved to. A leased profile whose
  toolchain lives elsewhere (a rustup sysroot, say) needs that path leased.
- Cancelling a leased goal whose task never opened no longer fails when the
  lease cannot confine its pool: nothing in such a task ever runs.

---

## Permission leases
**2026-10-24**

- A grant can carry a lease of read and write paths and an end. A child's
  lease must fit inside its parent's, on delegation, re-grant and restore.
- `lease: {read, write, ttl_secs}` in the hosting configuration confines a
  hosted goal: every assigned pool member must be sandboxed, and its sandbox
  keeps only what the lease also reaches. Lease paths must be canonical, and
  no member's path may lie beneath any member's write root. A run under a
  different lease is refused; one whose lease ended stops driving the task,
  and every harness is killed at the end by BBP itself, moderator or not.

---

## Factory execution-link hardening
**2026-10-21**

- Card execution-link adoption is persistence-idempotent, including concurrent
  submission and reopening the durable board store.
- Assignment delegation now reserves aggregate descendant and call budgets,
  rejects identity reuse and cycles, validates replacement grants, and fences
  descendants to the generation of the ancestor that delegated them.
- An authorized replacement generation can recover a non-terminal execution
  and persist it through Millboard's generation compare-and-set
  (`card_execution_recover`); stale generations can neither reconcile nor
  overwrite it.
- A parent's current generation re-grants a child stranded by the parent's
  replacement (`Factory::regrant`). Budget arithmetic is fully checked.

---

## Revision-bound change impact
**2026-10-20**

- Bundled inspection source travels over stdin, avoiding Windows process-launch error 206 while preserving Python isolation and process limits.

- `inspect --impact request.json` reports direct/transitive consumers, logical applications, binding requirements and required validation without execution.
- Complete RemindMe inventories preserve inactive contracts; revision comparison detects edits and withdrawals. Millboard ownership supports multiple projects per repository.
- Unknown consumers broaden validation. Reports bind clean source and input revisions; snapshot freshness requires refreshing exports.
- Requirement-linked projects contribute all owned Cargo packages to validation without including unrelated projects.
- Cargo metadata fingerprints normalize Cargo's unordered collections without masking dependency, feature, target, manifest or application-defined metadata changes.

---

## Autonomous maintenance execution
**2026-10-18**

- `maintain` defaults to a read-only plan. Explicit execution opens or recovers
  one bounded maintenance card using configured BBP providers and independent review.
- Human gates, Linux confinement, scoped manifest semantics and frozen Cargo
  validation remain mandatory. An explicit publication flag can open a draft PR
  for the exact validated tree. Maintenance never merges.
- Initial repairs cover layer metadata and human-confirmed dependency hoists.

---

## BBP provider launchers
**2026-10-17**

- Required Linux CI executes the pause/adoption/ledger regression separately
  from confined workloads. Deadline expiry is covered with live harness cleanup.
- No-progress launcher loops pause across restarts; owner reset is revision-fenced.
  Codex MCP approvals are scoped per tool, inherited Git context is scrubbed,
  and an explicit Rust/LLD profile bounds linker threads before task creation.

- The example roster now uses shipped Codex author and Claude reviewer launchers.
  Author turns use private clones; reviewer turns have no built-in tools and only
  their bound BBP MCP server. Startup validates active turn, principal and policy.
- Added offline contracts and an opt-in Linux subscription-provider smoke through
  validation, independent review, approval and a test-only receipt without merging.

---

## Required confined BBP acceptance
**2026-10-17**

- CI now requires the real Orch CLI lifecycle under production Linux confinement,
  including persisted resume, moderator restart, independent scripted review,
  merge approval and recorded receipt without performing a merge.
- Timeout, cancel and kill recovery have separate strict tests and retained
  per-mode results. Sandbox refusal or missing execution evidence fails the job.

---

## BBP task hosting implemented
**2026-10-16** · Issue #767

- Review corrections isolate task work, re-prompt rejected input, preserve expired requests, and retain runner ownership through cancellation/restart.
- Added `TaskHost` and `orch-bbp`, opt-in goal/CLI configuration, durable handles,
  frozen pool/roster/profile inputs, moderator supervision and crash adoption.
- Human gates persist revision/subject fences; stable operations replay after a
  lost checkpoint. Completion requires a recorded receipt, not merge approval.
- Existing research/design and one-call adapter paths remain available.
- Linux sandboxing stays mandatory in production; Windows parity and real
  provider/login validation remain outstanding. See `crates/orch-bbp/README.md`.

---

## BBP task host designed: ADR-0013
**2026-10-10** · (link once pushed) · [ADR-0013](docs/adr/0013-bbp-task-host.md)

- **Added (docs):** ADR-0013, Proposed, the design for roadmap Phase 1. A `Role::Implement` card whose goal names a repository is hosted on BBP: a second dispatcher port `TaskHost` (`open` is the one metered call, `poll` is free, `relay` passes a human answer through), `Progress::{Working, Gate, Closed, Cancelled}`, and `Outcome::Waiting` so a process can exit while agents work. The handle lives on the Board as an `Artifact` entry, so the snapshot persists it and `orch-core` is unchanged. BBP gates become Board `Question`s whose answers are `bbp human` verb lines relayed with the card revision, so a stale answer is refused rather than applied. The new `orch-bbp` crate drives the `bbp` binary over `orch_cli::CommandRunner` and decodes `bbp card` with the libs crate `rusty_bbp`; it never links `rusty_bbp_host` (cross-family apps dependency). Reviewer vendor is `Routing::reviewer(author)`, never the Coder's.
- **Rejected:** BBP as an `AgentRunner` (one call is not one task), the dispatcher proxying the CLI adapters into BBP (the relay loop BBP replaced), linking the host crate.
- **Changed (same day, after design review):** `open` is open-or-adopt and crash-recoverable (count and handle reach the snapshot together, every external step idempotent); the gate fence (card revision and the ids shown) is persisted in the Board `Question`'s refs and the relay holds the answer to it, never to a later poll; `poll` reads a new `bbp show` (card plus gate proposal, request body, receipt, cancellation reason) because the `Card` lacks them; `approved` is a gate ("merge outside BBP, then `receipt`"), never `Working`; every `bbp` reply is decoded as a `Response` since refusals print JSON and exit 0; a `Supervisor` seam owns the detached moderator because `CommandRunner` waits and kills at its deadline.
- Known limitation: design only. Phasing is five PRs: `bbp show`, port and fakes, adapter with supervisor and e2e, goal file and flags, Board slimming docs.

---

## Gemini adapter dropped
**2026-10-05** · (link once pushed)

- **Changed:** docs only. The Gemini CLI is discontinued, so the planned Gemini adapter is dropped, not deferred (README, ARCHITECTURE, AGENTS.md, ADR-0001 update, ADR-0012). `Agent::Gemini` stays a routing label so persisted records remain readable; no code change.
- Known limitation: a goal that routes to `gemini` still parses and fails at run time with "no adapter".

---

## Claude joins the board: `orch-claude`
**2026-10-04** · (link once pushed) · [ADR-0012](docs/adr/0012-claude-adapter.md)

- **Added:** `orch-claude`, `Agent::Claude` over the Claude Code CLI in print mode, read-only by three independent CLI layers (`--tools Read,Grep,Glob`, `--restricted`, `--permission-prompts none` under `dontAsk`), no settings files, MCP servers, skills, or session files, the repository root as the working directory, the reply constrained to the shared schema through `--json-schema` and read from the result envelope's `structured_output`, then parsed by `orch_cli::parse` as every reply is. `claude auth status` runs before each card; a non-zero exit is the unavailable "not logged in" prerequisite (not metered, card resumable). `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`, and `CLAUDE_CODE_USE_VERTEX` are scrubbed so the subscription login is the only path; `--bare` is not used because it never reads that login.
- **Added:** `rusty_orch run --repo <dir>` (env `ORCH_REPO`) names the repository Codex and Claude read; `--codex-repo` and `ORCH_CODEX_REPO` remain as aliases. `--claude-model <name>` pins a model. Goal files reach Claude by naming `claude` in `routing`; default routing is unchanged.
- **Changed:** `orch_cli::CommandRunner` gains the required `run_in(cwd, …)`, with `run_scrubbed` and `run` provided over it, so an adapter can give its CLI the repository as working directory instead of inheriting the orchestrator's. `orch_cli::output_schema(stop)` is the schema's home; `orch_codex::output_schema` re-exports it, so no caller changes. In-family source changes: every `CommandRunner` implementation (all in this family) implements `run_in`; `AgentsConfig.codex_repo`/`Args.codex_repo` become `repo` and both gain `claude_model`.
- **Tests:** orch-claude contract (argv pinned including the schema text and the absence of every write, shell, or bypass flag; constants; footer) and adapter over a scripted two-process fake (probe then run in the repo with the scrub list on both; logged-out probe is unavailable and skips the run; probe spawn failure is transient; login text inside the envelope is unavailable; four rate-limit wordings named; other envelope errors keep status and subtype; non-envelope exit reports stderr; prose, missing `structured_output`, and off-protocol objects are transient; timeout; best effort withdraws `question` from prompt, schema, and parser); orch-cli `run_in` sets the working directory and `None` inherits it, and the moved schema agrees with the parser under both rules; rusty_orch composite runner serves Codex, Claude, and Local and fails Gemini permanently; args cover `--repo`, the alias, and env precedence. Verified locally on rustc 1.97; CI pins 1.98.1. One live `claude -p` run with the pinned argv against this repository returned a schema-valid `structured_output` in three turns with no permission denials; the ignored real-binary nonce test is the standing check. The logged-out envelope text was not observed (the development container is always logged in); the probe's exit code is the load-bearing signal.
- Out of scope, by choice: the Gemini adapter (later dropped: the Gemini CLI is discontinued), the Implement role, write access, an OS sandbox for Claude, `--add-dir`.

---

## Best effort runs without a human
**2026-10-04** · (link once pushed)

- **Added:** `"stop": "best_effort"` is enforced. The shared adapter core withdraws the `question` kind from Research, Triage, and Design cards, the prompt gains two rules (nobody will answer, so do not ask; write an `assumption` entry stating what you take as true, then continue), the strict parser rejects a reply that asks anyway as a transient agent error retried under the card's ceiling, and Codex's structured-output schema omits the `question` variant so a constrained model cannot form one. Under `checkpoint` every prompt, schema, and parse result is byte-identical to before.
- **Added:** the text report lists live assumptions in an `assumptions (taken as true, unverified)` block when any exist; the JSON report already carried them as `kind: "assumption"`.
- **Changed:** `CodexAgent` and `OllamaAgent` take the rule through `with_stop_rule` (default `Checkpoint`); the binary parses the goal file first and builds the adapters with its rule (`cli::run_with`; `cli::run` keeps its four-argument form). In-family signature changes: `orch_cli::allowed_kinds`, `format_spec`, `parse` gain a `StopRule` parameter; `orch_ollama::render` and `orch_codex::render` likewise; `orch_codex::output_schema(stop)` replaces the `OUTPUT_SCHEMA` constant. No change to `orch-core`, `orch-dispatch`, the board, or the run loop.
- **Tests:** orch-cli parser (question rejected and assumption accepted under best effort, review and implement unchanged) and format spec (two rules added, kinds line without `question`, review spec identical under both rules); orch-ollama and orch-codex adapters over fakes (best-effort prompt carries the rules, a question reply is a transient error, the same reply is a question under checkpoint; Codex schema strict under both rules and without a `question` variant under best effort, and in agreement with the parser); rusty_orch run loop (a best-effort goal over an assumption-writing fake finishes without asking the console and the report shows the block; the block is absent otherwise) and entry point (`run_with` receives the goal's rule). Verified locally on rustc 1.97; CI pins 1.98.1. No real Codex or Ollama. ADR-0011.
- Out of scope, by choice: an orchestrator-authored assumption, a dispatcher-side fallback, per-card overrides, Claude and Gemini adapters, per-step checkpointing.

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
